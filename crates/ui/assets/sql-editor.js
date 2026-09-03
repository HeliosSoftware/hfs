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
 * on) and its HighlightStyle. No fold, no lint, no
 * autocomplete yet (#839/#842).
 *
 * The Library JSON pane on the same page (`textarea[name="json"]`, behind
 * the fold) is untouched - that is #840.
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
      // SpecialVar (`:ward`, SQLite dialect) - the token the epic's dialect
      // decision exists for.
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

  CodeEditor.mount(textarea, {
    language: sqlLanguage,
    highlight: CM.syntaxHighlighting(sqlHighlightStyle),
    fold: false,
    wrapperClass: "sql-editor",
    id: "sql-editor",
  });
})();
