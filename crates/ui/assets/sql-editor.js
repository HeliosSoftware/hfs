/*
 * SQL editor mount for SQL Queries / SQL Views (#838).
 *
 * Progressive enhancement over the plain `<textarea name="sql">` in
 * `#lib-editor-form` on `/ui/sql/queries` and `/ui/sql/views` (both served
 * by the same `sql-library.html` template): if the vendored CodeMirror 6
 * bundle (`/ui/assets/vendor/codemirror.bundle.js`, global
 * `window.HfsCodeMirror`) and the shared mount helper (`code-editor.js`,
 * global `window.HfsCodeEditor`) both loaded and that textarea
 * exists, this mounts a CodeMirror 6 editor with SQL syntax highlighting on
 * top of it.
 *
 * The wrapper/sync/back-out plumbing (textarea as source of truth, Tab not
 * captured, aria-label, silent degradation without the bundle or on any
 * construction error) belongs to `code-editor.js` and is not duplicated
 * here - this file only owns what is specific to the SQL editor: the
 * language (SQLite dialect, the engine `$sqlquery-run` actually executes
 * on), its HighlightStyle, the failed-line tint, and the unknown-table
 * lint's own diagnostics/gutter (#842/04) below. No fold, no
 * autocomplete yet (follow-up A of #842).
 *
 * The Library JSON pane on the same page (`input[type="hidden"][name="json"]`,
 * no fold since #839) is untouched - #840 replaces it with a Details view.
 *
 * Without the bundle, without the helper, without a `#lib-editor-form`
 * (the page's empty state has none), without JS, or if anything below
 * throws, this file does nothing and the page is the textarea as it is
 * today.
 */
(function () {
  "use strict";

  var CodeEditor = window.HfsCodeEditor;
  var CM = window.HfsCodeMirror;
  var form = document.getElementById("lib-editor-form");
  var textarea = form ? form.querySelector('textarea[name="sql"]') : null;
  if (!CodeEditor || !CM || !textarea) return;

  var sqlLanguage = CM.sql({ dialect: CM.SQLite });

  /* ---- Highlighting: classes only - every color lives in app.css as a
   * CSS variable, never a fixed value from here. Tag -> class
   * mapping cross-checked against @codemirror/lang-sql's own styleTags()
   * call (node_modules/@codemirror/lang-sql/dist/index.js):
   *
   *   Keyword -> keyword, Type -> typeName, Bool -> bool, Null -> null,
   *   Number/Bits -> number, String/Bytes -> string,
   *   QuotedIdentifier -> special(string), SpecialVar -> special(name),
   *   LineComment/BlockComment -> (sub-tags of) comment, Operator -> operator,
   *   "Semi Punctuation" -> punctuation, "( )" -> paren, "{ }" -> brace,
   *   "[ ]" -> squareBracket.
   *
   * Identifier (bare column/table names) and Builtin (built-in function
   * names, tags.standard(name)) are deliberately left without a rule -
   * nine token groups get a color below, not schema-dependent names; they
   * render in the editor's default text color. QuotedIdentifier is a
   * lezer-highlight sub-tag of string, so it falls back to the string rule
   * below rather than needing its own.
   */
  var sqlHighlightStyle = CM.HighlightStyle.define(
    [
      { tag: CM.tags.keyword, class: "cmt-sql-keyword" },
      { tag: CM.tags.typeName, class: "cmt-sql-type" },
      { tag: CM.tags.string, class: "cmt-sql-string" },
      { tag: CM.tags.number, class: "cmt-sql-number" },
      { tag: CM.tags.operator, class: "cmt-sql-operator" },
      // SpecialVar (`:ward`, SQLite dialect) - a bound parameter token.
      { tag: CM.tags.special(CM.tags.name), class: "cmt-sql-variable" },
      { tag: CM.tags.comment, class: "cmt-sql-comment" },
      {
        tag: [CM.tags.punctuation, CM.tags.paren, CM.tags.brace, CM.tags.squareBracket],
        class: "cmt-sql-punct",
      },
      { tag: [CM.tags.bool, CM.tags.null], class: "cmt-sql-literal" },
    ],
    { scope: sqlLanguage.language }
  );

  /* ---- Failed-line tint (#839): `partials/sql_run_results.html` marks a
   * `$sql-run` parse failure's notice with `data-error-line="N"`
   * (`sql_views::extract_error_line` - only sqlparser errors carry a line;
   * SQLite execution errors don't) each time htmx swaps `#run-notice`, both
   * from the live-run debounce on the textarea below and from the page's
   * own initial load. This is purely a decoration on top of the document
   * CodeMirror already has - it never edits `doc`, moves the cursor, or
   * touches the textarea/dispatches `input`, so it cannot feed back into
   * the 500ms live-run loop.
   *
   * `errorLineEffect.of(n)` (1-based line number) replaces any previous
   * tint with one on line `n`; `.of(null)` clears it. A `StateField` is the
   * idiomatic CodeMirror 6 way to hold "at most one decoration, changed
   * only by an explicit effect" - see the state.md line-highlight recipe
   * this mirrors. */
  var errorLineEffect = CM.StateEffect.define();
  var errorLineMark = CM.Decoration.line({ attributes: { class: "sql-editor__error-line" } });
  var errorLineField = CM.StateField.define({
    create: function () {
      return CM.Decoration.none;
    },
    update: function (decorations, tr) {
      decorations = decorations.map(tr.changes);
      for (var i = 0; i < tr.effects.length; i++) {
        var effect = tr.effects[i];
        if (!effect.is(errorLineEffect)) continue;
        decorations =
          effect.value == null
            ? CM.Decoration.none
            : CM.Decoration.set([errorLineMark.range(tr.state.doc.line(effect.value).from)]);
      }
      return decorations;
    },
    provide: function (field) {
      return CM.EditorView.decorations.from(field);
    },
  });

  var view = CodeEditor.mount(textarea, {
    language: sqlLanguage,
    highlight: CM.syntaxHighlighting(sqlHighlightStyle),
    extensions: [errorLineField, CM.lintGutter()],
    fold: false,
    wrapperClass: "sql-editor",
    id: "sql-editor",
  });
  if (!view) return;

  /* ---- Unknown-table diagnostics (#842/04): `partials/lib_run_unknown_
   * tables.html` marks its own notice with `data-diagnostics` - a JSON
   * array of `{ from, to, message, table }` objects, character offsets
   * into the SQL text as posted. `applyDiagnostics` turns that into
   * `@codemirror/lint`'s own `Diagnostic[]` shape (`severity: "error"`,
   * `source: "unknown-table"`) and hands it to `setDiagnostics`, which
   * `lintGutter()` above renders as an underline with a hover tooltip and a
   * mark in the gutter. `from`/`to` are clamped to the document's current
   * length - the posted SQL and the editor's own text are normally
   * identical, but a swap can in principle land a moment after the
   * visitor kept typing. Missing or unparseable `data-diagnostics` (every
   * other notice - a good run, a plain execution failure, "waiting for a
   * value") clears whatever the previous notice set, exactly like
   * `errorLineEffect.of(null)` above. */
  function applyDiagnostics(target) {
    var el = target.querySelector("[data-diagnostics]");
    var raw = el ? el.getAttribute("data-diagnostics") : null;
    var diagnostics = [];
    if (raw) {
      try {
        var parsed = JSON.parse(raw);
        var docLength = view.state.doc.length;
        if (Array.isArray(parsed)) {
          diagnostics = parsed.map(function (d) {
            var from = Math.max(0, Math.min(docLength, d.from | 0));
            var to = Math.max(from, Math.min(docLength, d.to | 0));
            return {
              from: from,
              to: to,
              severity: "error",
              source: "unknown-table",
              message: String(d.message || ""),
            };
          });
        }
      } catch (e) {
        diagnostics = [];
      }
    }
    view.dispatch(CM.setDiagnostics(view.state, diagnostics));
  }

  /* `#run-notice` is always swapped wholesale (`hx-swap="outerHTML"`, both
   * the live-run trigger and the page's own initial load in
   * `sql-library.html`), so htmx dispatches this `htmx:afterSwap` on the
   * new `#run-notice` element itself - `event.target` is already the
   * settled replacement, no need to re-query the DOM by id. Registered
   * once, for the page's whole lifetime: it does not re-subscribe on every
   * swap and holds no reference beyond `view`. */
  document.addEventListener("htmx:afterSwap", function (event) {
    var target = event.target;
    if (!target || target.id !== "run-notice") return;
    var notice = target.querySelector("[data-error-line]");
    var line = notice ? parseInt(notice.getAttribute("data-error-line"), 10) : NaN;
    var inRange = Number.isInteger(line) && line >= 1 && line <= view.state.doc.lines;
    view.dispatch({ effects: errorLineEffect.of(inRange ? line : null) });
    applyDiagnostics(target);
  });
})();
