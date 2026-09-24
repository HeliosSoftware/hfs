/*
 * The "+ Add Element" picker (#1239), shared by the three editor hosts:
 * the standalone Resource Editor (`editor.js`), the Resources workspace
 * modal (`resources.js`), and the `pane=form` guided form embedded in the
 * View Definition / Library editors (`editor-form.js`). Each of those hosts
 * re-renders its own container from the server on every mutation, so
 * without this module each one carried its own copy of:
 *
 *   - capturing which `details.editor-add` picker was open (and its filter
 *     text/focus) before the swap, and reopening it afterwards;
 *   - reading the extension URL a `[data-extension]` click should add,
 *     whether it came from the clicked button's own `data-url` or a typed
 *     `.editor-add__ext-url` field;
 *   - the picker's filter typeahead, hiding `[data-add-name]` entries that
 *     do not match what was typed.
 *
 * `window.HfsEditorAdd`:
 *   - `capturePickers(container)` -> the open pickers inside `container`,
 *     to hand back to `restorePickers` after the next re-render.
 *   - `restorePickers(container, pickers)` -> reopens each one, restoring
 *     its filter text and, where it had it, focus. Does not move the
 *     caller's own focus to whatever node the mutation created — that
 *     stays the host's job, run after this call returns.
 *   - `extensionUrl(button)` -> the URL a `[data-extension]` click should
 *     send, from the button itself or the panel's typed field.
 *   - `attach(container)` -> installs the filter typeahead once per
 *     container (a repeat call on the same container is a no-op, guarded
 *     via `container.dataset.hfsEditorAdd`).
 *   - `matches(name, needle)` -> the typeahead's own match rule, exported
 *     as a pure function for its unit test
 *     (`crates/ui/e2e/unit/editor-add.test.cjs`).
 *
 * Same UMD-ish shape as `editor-pair.js`: `attach` only ever runs from a
 * real page, so requiring this file under Node defines the functions and
 * does nothing else.
 */
(function (root, factory) {
  "use strict";

  var api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.HfsEditorAdd = api;
})(typeof window !== "undefined" ? window : null, function () {
  "use strict";

  /* The row for `path` inside `container` — the container itself for the
   * root path (`""`, no row of its own), otherwise the `[data-path]`
   * descendant whose own path matches. */
  function rowByPath(container, path) {
    if (!path) return container;
    var rows = container.querySelectorAll("[data-path]");
    for (var i = 0; i < rows.length; i++) {
      if (rows[i].dataset.path === path) return rows[i];
    }
    return null;
  }

  function capturePickers(container) {
    var pickers = [];
    container.querySelectorAll("details.editor-add[open]").forEach(function (box) {
      var row = box.closest("[data-path]");
      var filter = box.querySelector(".editor-add__filter");
      pickers.push({
        path: row ? row.dataset.path : "",
        filter: filter ? filter.value : "",
        focusFilter: filter === document.activeElement,
      });
    });
    return pickers;
  }

  function restorePickers(container, pickers) {
    (pickers || []).forEach(function (saved) {
      var row = rowByPath(container, saved.path);
      if (!row) return;
      var box = row.querySelector("details.editor-add");
      if (!box) return;
      box.setAttribute("open", "");
      var filter = box.querySelector(".editor-add__filter");
      if (filter && saved.filter) {
        filter.value = saved.filter;
        filter.dispatchEvent(new Event("input", { bubbles: true }));
      }
      if (saved.focusFilter && filter) filter.focus();
    });
  }

  function extensionUrl(button) {
    if (button.dataset.url) return button.dataset.url;
    var panel = button.closest(".editor-add__ext");
    if (!panel) return "";
    return panel.querySelector(".editor-add__ext-url").value.trim();
  }

  /* Case-insensitive substring match, empty needle matching everything —
   * the typeahead's own rule, exported so its unit test does not need a
   * DOM to exercise it. */
  function matches(name, needle) {
    if (!needle) return true;
    return name.toLowerCase().indexOf(needle.toLowerCase()) !== -1;
  }

  function attach(container) {
    if (!container || container.dataset.hfsEditorAdd === "1") return;
    container.dataset.hfsEditorAdd = "1";
    container.addEventListener("input", function (event) {
      var filter = event.target.closest(".editor-add__filter");
      if (!filter) return;
      var needle = filter.value.trim().toLowerCase();
      var panel = filter.closest(".editor-add__panel");
      panel.querySelectorAll("[data-add-name]").forEach(function (item) {
        item.hidden = !matches(item.dataset.addName, needle);
      });
    });
  }

  return {
    capturePickers: capturePickers,
    restorePickers: restorePickers,
    extensionUrl: extensionUrl,
    attach: attach,
    matches: matches,
  };
});
