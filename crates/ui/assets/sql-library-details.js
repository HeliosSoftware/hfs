/*
 * Details JSON editor mount for SQL Queries / SQL Views (#840).
 *
 * Progressive enhancement over the plain `<textarea class="json-editor"
 * name="json" form="lib-editor-form">` in the Details JSON card on
 * `/ui/sql/queries` and `/ui/sql/views` (both served by `sql-library.html`):
 * if the vendored CodeMirror 6 bundle (`/ui/assets/vendor/codemirror.bundle.js`,
 * global `window.HfsCodeMirror`) and the shared mount helper (`code-editor.js`,
 * global `window.HfsCodeEditor`) both loaded and that textarea exists, this
 * mounts a CodeMirror 6 editor with JSON syntax highlighting on top of it —
 * the shared preset `code-editor.js` exposes (`jsonHighlight()`), the same
 * one `vd-editor.js` uses for its own outer JSON grammar.
 *
 * The wrapper/sync/back-out plumbing (textarea as source of truth, Tab not
 * captured, aria-label, silent degradation without the bundle or on any
 * construction error) belongs to `code-editor.js`; driving the guided-form
 * card beside the editor — the JSON<->form sync, the validity chip, and the
 * row<->editor cross-highlight — belongs to the shared host `editor-pair.js`
 * (#840, the same one View Definitions uses). This file only owns the
 * mount call and the two `fields` that tell the shared guided-form loop
 * (`editor-form.js`) which document it is editing: `hidden: "content"` (the
 * SQL card below owns that attachment, never the Details panel) and
 * `legend: "sql-library"` (the two-line "checked as you type"/"checked on
 * save" legend `crate::render_lib_details_pane` already built the page's
 * own first paint with — see `editor::Legend`).
 *
 * The SQL pane on the same page (`#lib-editor-form textarea[name='sql']`,
 * `sql-editor.js`) is a separate document entirely and untouched here.
 *
 * `EditorPair.mount`'s own return value — `{formApi, host}` — is kept and
 * exposed as `window.HfsSqlLibraryDetails` (#841), the one point a page
 * script reaches the Details pairing through: `host.setDoc(text)` applies a
 * document-mutation endpoint's `data-document` response as one undoable
 * transaction (Ctrl+Z restores the pre-mutation text) that also refreshes
 * the guided form and, via the textarea's own `input` event, re-fires the
 * live run — see `sql-library-panels.js`. `undefined` when `EditorPair.mount`
 * itself did nothing (no bundle, no `editor-pair.js`, no textarea).
 *
 * Without the bundle, without the helper, without the textarea, without
 * `editor-pair.js`, without JS, or if anything below throws, this file does
 * nothing (or — no CodeMirror, but `editor-pair.js` present — hands it the
 * plain textarea to drive instead, exactly like `vd-editor.js` does for its
 * own editor).
 */
(function () {
  "use strict";

  var CodeEditor = window.HfsCodeEditor;
  var CM = window.HfsCodeMirror;
  var EditorPair = window.HfsEditorPair;

  var textarea = document.querySelector(
    'textarea[name="json"][form="lib-editor-form"]',
  );
  if (!textarea || !EditorPair) return;

  var grid = document.getElementById("lib-details-grid");
  var view = null;

  if (CodeEditor && CM) {
    // No `wrapperClass` — nothing here needs a class of its own to select
    // by: sizing already comes from the shared `.editor__doc .cm-editor`
    // rule (app.css, #843/#840) the SQL editor's own `.sql-editor` and the
    // ViewDefinition editor's own `.vd-editor` need a dedicated class for
    // (their own token-color/FHIRPath rules), which this plain JSON pane
    // does not have. `id` alone gives tests a stable hook.
    view = CodeEditor.mount(textarea, {
      language: CM.json(),
      highlight: CM.syntaxHighlighting(CodeEditor.jsonHighlight()),
      fold: true,
      id: "lib-details-editor",
    });
  }

  // Missing `grid` (a page fragment other than this one's own template) or
  // a missing `editor-form.js` load leaves the editor above exactly as it
  // is and touches nothing else — `EditorPair.mount` guards both itself.
  window.HfsSqlLibraryDetails = EditorPair.mount({
    textarea: textarea,
    view: view,
    grid: grid,
    fields: { hidden: "content", legend: "sql-library" },
  });
})();
