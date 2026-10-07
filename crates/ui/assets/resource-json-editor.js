/*
 * The resource editor's JSON pane (#1756), shared by the Resource Editor page
 * (`editor.js`) and the Resources modal (`resources.js`).
 *
 * The pane is always the shared code editor: `code-editor.js` mounts
 * CodeMirror 6 over `#editor-source` (JSON highlighting, line numbers,
 * folding, Format, Tab leaving the editor), and `editor-pair.js` keeps it and
 * the guided form beside it showing the same document in both directions,
 * with `editor-form.js` running the form's own round trips. There is no
 * read-only view and no "Edit raw" toggle. Without the vendored bundle the
 * same textarea stays visible and `editor-pair.js` drives it instead.
 *
 * `window.HfsResourceJsonEditor`:
 *
 *   mount(body)          -> `{ view, pair, textarea }`, or `null` when `body`
 *                           holds no `#editor-source` / `.editor__grid` (an
 *                           invalid-JSON notice rendered instead of the
 *                           editor). `body` is the container the server's
 *                           `partials/editor-body.html` was rendered into
 *                           (`#editor-body`, `#resource-editor-body`); it is
 *                           also the guided-form root handed to
 *                           `editor-pair.js`, because the hidden state and the
 *                           datalists the form swaps sit next to the grid, not
 *                           inside it. `view` is the `EditorView` (`null`
 *                           without the bundle), `pair` is `editor-pair.js`'s
 *                           `{ formApi, host }` (`null` without that script).
 *                           Mount a given element once: the form loop
 *                           attaches its listeners to it.
 *
 *   render(body, text, isStale) -> Promise of `{ body, session }`: posts `text` to
 *                           `/ui/editor/render`, puts the whole rendered body
 *                           into a fresh element that replaces `body` (so no
 *                           listener of an earlier mount survives), and mounts
 *                           it. `session` is `mount`'s result, or an empty
 *                           one (all `null`) for an invalid document. Resolves
 *                           `null`, touching nothing, when the optional
 *                           `isStale()` is true once the response arrives (a
 *                           newer render superseded this one). Rejects when
 *                           the request or the response fails.
 *
 *   apply(session, text) -> Promise of boolean: `text` becomes the editor's
 *                           document as one undoable transaction (Ctrl+Z
 *                           restores what was there) and the guided form is
 *                           refreshed for it. True when the form now shows it.
 *
 *   project(session, text) -> Promise of boolean: refreshes the guided form
 *                           for `text` without touching the editor — the
 *                           validation render a Save runs before writing. True
 *                           when the form now shows `text`.
 *
 *   destroy(body)        -> tears down the code editor mounted in `body` (its
 *                           window/document listeners); `render` does this for
 *                           the body it replaces.
 *
 * `Collapse all` / `Expand all` in the card head are revealed here and run the
 * fold-all / unfold-all commands the bundle's `foldKeymap` carries on
 * `Ctrl-Alt-[` / `Ctrl-Alt-]`; the bundle does not export them as symbols.
 * Focus stays on the button.
 *
 * Without `editor-pair.js`, without the bundle, or if the editor cannot be
 * built, the page keeps working on the plain textarea.
 */
(function () {
  "use strict";

  var RENDER_URL = "/ui/editor/render";

  function canonical(text) {
    try { return JSON.stringify(JSON.parse(text)); } catch (invalid) { return null; }
  }

  function foldCommand(CM, key) {
    var keymap = CM && CM.foldKeymap;
    if (!keymap) return null;
    for (var i = 0; i < keymap.length; i++) {
      if (keymap[i].key === key && typeof keymap[i].run === "function") return keymap[i].run;
    }
    return null;
  }

  function wireFoldButtons(card, view) {
    var CM = window.HfsCodeMirror;
    [["all", "Ctrl-Alt-["], ["none", "Ctrl-Alt-]"]].forEach(function (entry) {
      var button = card && card.querySelector('[data-editor-fold="' + entry[0] + '"]');
      var run = foldCommand(CM, entry[1]);
      if (!button || !run) return;
      button.hidden = false;
      button.addEventListener("click", function () { run(view); });
    });
  }

  function mount(body) {
    if (!body) return null;
    var textarea = body.querySelector("#editor-source");
    var grid = body.querySelector(".editor__grid");
    if (!textarea || !grid) return null;

    var view = null;
    var CodeEditor = window.HfsCodeEditor;
    var CM = window.HfsCodeMirror;
    if (CodeEditor && CM) {
      view = CodeEditor.mount(textarea, {
        language: CM.json(),
        highlight: CM.syntaxHighlighting(CodeEditor.jsonHighlight()),
        fold: true,
        format: "json",
        wrapperClass: "code-editor--resource",
      }) || null;
    }
    if (view) wireFoldButtons(textarea.closest(".card"), view);

    var pair = null;
    if (window.HfsEditorPair) {
      pair = window.HfsEditorPair.mount({
        textarea: textarea,
        view: view,
        grid: body,
        invalidJsonMessage: grid.dataset.msgJsonInvalid || "",
      }) || null;
    }
    return { view: view, pair: pair, textarea: textarea, body: body };
  }

  function project(session, text) {
    if (!session || !session.pair) return Promise.resolve(false);
    var body = session.body;
    return Promise.resolve(session.pair.formApi.refresh(text)).then(function () {
      var field = body.querySelector("#editor-doc");
      var shown = field ? canonical(field.value) : null;
      return shown !== null && shown === canonical(text);
    });
  }

  function apply(session, text) {
    if (!session || !session.pair) return Promise.resolve(false);
    session.pair.host.setDoc(text);
    return project(session, text);
  }

  /* Tears down the code editor mounted in `body`, if any: CodeMirror keeps
   * listeners on `window` and `document` that only `destroy()` removes. */
  function destroy(body) {
    var CM = window.HfsCodeMirror;
    var content = body && body.querySelector(".cm-content");
    if (!CM || !content) return;
    var view = CM.EditorView.findFromDOM(content);
    if (view) view.destroy();
  }

  function render(body, text, isStale) {
    var form = new URLSearchParams();
    form.set("doc", text);
    form.set("op", "");
    return fetch(RENDER_URL, { method: "POST", body: form })
      .then(function (response) {
        if (!response.ok) throw new Error(String(response.status));
        return response.text();
      })
      .then(function (html) {
        if (isStale && isStale()) return null;
        var fresh = document.createElement(body.tagName);
        fresh.id = body.id;
        fresh.className = body.className;
        fresh.innerHTML = html;
        if (!fresh.querySelector("#editor-form")) throw new Error("Invalid editor render response");
        destroy(body);
        body.replaceWith(fresh);
        var session = mount(fresh) || { view: null, pair: null, textarea: null, body: fresh };
        return { body: fresh, session: session };
      });
  }

  window.HfsResourceJsonEditor = { mount: mount, render: render, destroy: destroy, apply: apply, project: project };
})();
