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
 *   sameDocument(left, right) -> true when both texts are JSON and equal as
 *                           documents (object key order ignored, array order
 *                           kept); the linter's check that the panel is current.
 *
 *   issueDiagnostics(issues, resolve) -> the validation issues as lint
 *                           diagnostics (pure; unit-tested under Node). Each
 *                           `{path, message}` is a `warning` on `resolve(path)`
 *                           (`{from, to}` or null), climbing to the nearest
 *                           ancestor that resolves when the element is
 *                           missing; the root is `{from: 0, to: 1}`; the
 *                           message is `path: message`; repeats collapse.
 *
 * Line markers (#1756): the editor carries a lint source and the lint gutter.
 * A JSON syntax error is marked red on its line and is all that shows; for
 * valid JSON the server's issues, read from the `data-issues` of the guided
 * panel's `.editor-validity` (no request of its own), are marked amber on
 * their elements' lines with the message in the hover card and the lint panel.
 * They show only while the panel's `#editor-doc` is the same document as this text; a
 * MutationObserver re-lints when the panel is replaced and stops once the
 * editor leaves the document.
 *
 * Without `editor-pair.js`, without the bundle, or if the editor cannot be
 * built, the page keeps working on the plain textarea.
 */
(function (root, factory) {
  "use strict";

  var api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.HfsResourceJsonEditor = api;
})(typeof window !== "undefined" ? window : null, function () {
  "use strict";

  var RENDER_URL = "/ui/editor/render";

  /* The validation issues (`[{path, message}]`) as line diagnostics: each one
   * on the range `resolve(path)` gives, or, when the path does not resolve
   * (an element that is missing), on its nearest ancestor that does; the root
   * (`""`) is the document's first character. Warnings, since the document is
   * valid JSON; the message carries its path. Repeats collapse into one. */
  function issueDiagnostics(issues, resolve) {
    var out = [];
    var seen = {};
    (Array.isArray(issues) ? issues : []).forEach(function (issue) {
      if (!issue || typeof issue.message !== "string") return;
      var path = typeof issue.path === "string" ? issue.path : "";
      var segments = path === "" ? [] : path.split(".");
      var range = null;
      while (true) {
        if (segments.length === 0) { range = { from: 0, to: 1 }; break; }
        range = resolve(segments.join("."));
        if (range) break;
        segments.pop();
      }
      var message = path ? path + ": " + issue.message : issue.message;
      var key = range.from + ":" + range.to + ":" + message;
      if (seen[key]) return;
      seen[key] = true;
      out.push({ from: range.from, to: range.to, severity: "warning", message: message, source: "fhir" });
    });
    return out;
  }

  function deepEqual(a, b) {
    if (a === b) return true;
    if (a === null || b === null || typeof a !== "object" || typeof b !== "object") return false;
    if (Array.isArray(a) !== Array.isArray(b)) return false;
    var keysA = Object.keys(a);
    if (keysA.length !== Object.keys(b).length) return false;
    return keysA.every(function (key) {
      return Object.prototype.hasOwnProperty.call(b, key) && deepEqual(a[key], b[key]);
    });
  }

  /* True when both texts parse as JSON and are the same document: object key
   * order does not matter, array order does. */
  function sameDocument(left, right) {
    try { return deepEqual(JSON.parse(left), JSON.parse(right)); } catch (invalid) { return false; }
  }

  function readIssues(body) {
    var chip = body.querySelector(".editor-validity");
    if (!chip) return [];
    try {
      var issues = JSON.parse(chip.getAttribute("data-issues") || "[]");
      return Array.isArray(issues) ? issues : [];
    } catch (invalid) {
      return [];
    }
  }

  /* The editor's lint source: a syntax error on its line, red, and nothing
   * else; otherwise the server's validation issues for this very text (the
   * panel's `#editor-doc` must be the same document, else the panel has not
   * caught up yet) on the elements' lines, amber. No request of its own. */
  function makeLinter(CM, body) {
    var syntax = CM.jsonParseLinter();
    return function (view) {
      var errors = syntax(view);
      if (errors && errors.length) return errors;
      var text = view.state.doc.toString();
      var field = body.querySelector("#editor-doc");
      if (!field || !sameDocument(field.value, text)) return [];
      var pair = window.HfsEditorPair;
      if (!pair || !pair.rangeOfPath) return [];
      return issueDiagnostics(readIssues(body), function (path) {
        return pair.rangeOfPath(view.state, path);
      });
    };
  }

  function touchesIssues(node) {
    if (!node || node.nodeType !== 1) return false;
    return node.matches(".editor-validity, #editor-doc") ||
      !!node.querySelector(".editor-validity, #editor-doc");
  }

  /* Re-runs the linter whenever the panel's issues are replaced or changed. */
  function watchIssues(CM, view, body, refresh) {
    if (typeof MutationObserver === "undefined") return;
    var observer = new MutationObserver(function (mutations) {
      if (!document.contains(view.dom)) { observer.disconnect(); return; }
      var relevant = mutations.some(function (m) {
        if (m.type === "attributes") return true;
        return Array.prototype.some.call(m.addedNodes, touchesIssues);
      });
      if (!relevant) return;
      /* `forceLinting` only runs a lint that is already pending, so the
       * refresh flag (read by the linter's `needsRefresh`) is raised and an
       * empty transaction lets the lint plugin see it before it is forced. */
      refresh.pending = true;
      view.dispatch({});
      CM.forceLinting(view);
    });
    observer.observe(body, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["data-issues"],
    });
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
    var refresh = { pending: false };
    var CodeEditor = window.HfsCodeEditor;
    var CM = window.HfsCodeMirror;
    if (CodeEditor && CM) {
      view = CodeEditor.mount(textarea, {
        language: CM.json(),
        highlight: CM.syntaxHighlighting(CodeEditor.jsonHighlight()),
        fold: true,
        format: "json",
        wrapperClass: "code-editor--resource",
        extensions: [
          CM.linter(makeLinter(CM, body), {
            delay: 300,
            needsRefresh: function () {
              var due = refresh.pending;
              refresh.pending = false;
              return due;
            },
          }),
          CM.lintGutter(),
        ],
      }) || null;
    }
    if (view) {
      wireFoldButtons(textarea.closest(".card"), view);
      try { watchIssues(CM, view, body, refresh); } catch (unavailable) { /* markers refresh on the next edit */ }
    }

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
      return field ? sameDocument(field.value, text) : false;
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

  return {
    mount: mount,
    render: render,
    destroy: destroy,
    apply: apply,
    project: project,
    issueDiagnostics: issueDiagnostics,
    sameDocument: sameDocument,
  };
});
