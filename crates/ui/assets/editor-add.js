/*
 * The "+ Add Element" picker (#1239), shared by the three editor hosts:
 * the standalone Resource Editor (`editor.js`), the Resources workspace's
 * own edit surface (`resources.js`), and the `pane=form` guided form
 * embedded in the View Definition / Library editors (`editor-form.js`).
 * Each of those hosts re-renders its own container from the server on
 * every mutation, so without this module each one carried its own copy
 * of four things:
 *
 *   - state across the swap: capturing which `details.editor-add` picker
 *     was open (its filter text/focus, and whether its Extensions group was
 *     open) before the swap, and reopening it afterwards — including, when
 *     the mutation the swap answers created a node under that picker,
 *     clearing the filter and showing the "<name> added" signal instead of
 *     restoring it;
 *   - the filter typeahead and the Elements/Extensions group folding it
 *     drives, hiding `[data-add-name]` entries that do not match what was
 *     typed;
 *   - closing the panel — the × button, Escape (stopped before it reaches
 *     any other Escape handler a host installs of its own), and a click
 *     outside it — on top of the native `<details>` toggle on the summary;
 *   - reading the extension URL a `[data-extension]` click should add,
 *     whether it came from the clicked button's own `data-url` or a typed
 *     `.editor-add__ext-url` field.
 *
 * `window.HfsEditorAdd`:
 *   - `capturePickers(container)` -> the open pickers inside `container`,
 *     to hand back to `restorePickers` after the next re-render.
 *   - `restorePickers(container, pickers, createdPath)` -> reopens each
 *     one. `createdPath` (optional) is the path the last mutation created,
 *     from `#editor-form[data-focus]`; the picker that owns it (its own
 *     path is that node's parent) clears its filter and shows the "added"
 *     signal with Undo instead of restoring the filter text. Every other
 *     picker gets its filter text and, where it had it, focus back as
 *     before. Does not move the caller's own focus to whatever node the
 *     mutation created — that stays the host's job, run after this call
 *     returns.
 *   - `extensionUrl(button)` -> the URL a `[data-extension]` click should
 *     send, from the button itself or the panel's typed field.
 *   - `attach(container)` -> installs the filter typeahead and the close
 *     behaviors once per container (a repeat call on the same container is
 *     a no-op, guarded via `container.dataset.hfsEditorAdd`).
 *   - `matches(name, needle)`, `parentPath(path)`, `leafName(path)` -> pure
 *     helpers exported for their unit tests
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

  /* A trailing repetition index, in either spelling. The editor's own paths
   * are dotted with numeric segments (`name.0.given.2`, `extension.0` — see
   * `crates/ui/src/editor.rs`); the bracket form is accepted for callers
   * that spell them the FHIRPath way (`name[0].given[2]`). */
  var TRAILING_INDEX = /(\.\d+|\[\d+\])$/;

  /* `path` with a trailing repetition index stripped, and everything from
   * the last remaining `.` on dropped — the parent row's own `data-path`,
   * or `""` for a path with no parent (a top-level element, or the empty
   * root path itself). `name.1` and `extension.0` are top-level: their
   * parent is the root. */
  function parentPath(path) {
    if (!path) return "";
    var stripped = path.replace(TRAILING_INDEX, "");
    var dot = stripped.lastIndexOf(".");
    return dot === -1 ? "" : stripped.substring(0, dot);
  }

  /* `path` with a trailing repetition index stripped, and everything up to
   * and including the last remaining `.` dropped — the element name a
   * picker's "added" signal names (`name.1` → `name`). */
  function leafName(path) {
    var stripped = (path || "").replace(TRAILING_INDEX, "");
    var dot = stripped.lastIndexOf(".");
    return dot === -1 ? stripped : stripped.substring(dot + 1);
  }

  function capturePickers(container) {
    var pickers = [];
    container.querySelectorAll("details.editor-add[open]").forEach(function (box) {
      var row = box.closest("[data-path]");
      var filter = box.querySelector(".editor-add__filter");
      var extGroup = box.querySelector('details[data-add-group="extensions"]');
      var active = document.activeElement;
      pickers.push({
        path: row ? row.dataset.path : "",
        filter: filter ? filter.value : "",
        // Undo is about to vanish with the re-render; the filter is where
        // that user continues, so it inherits the focus (#1239).
        focusFilter: filter === active || !!(active && box.contains(active) && active.matches("[data-add-undo]")),
        // Only a group the user unfolded on their own is worth restoring:
        // with a filter typed, the typeahead decides, and once the filter
        // is cleared the group goes back to folded (#1239).
        extOpen: !!(extGroup && extGroup.open) && !(filter && filter.value.trim()),
      });
    });
    return pickers;
  }

  function restorePickers(container, pickers, createdPath) {
    (pickers || []).forEach(function (saved) {
      var row = rowByPath(container, saved.path);
      if (!row) return;
      var box = row.querySelector("details.editor-add");
      if (!box) return;
      box.setAttribute("open", "");
      var filter = box.querySelector(".editor-add__filter");
      var owns = !!createdPath && parentPath(createdPath) === saved.path;
      if (owns) {
        // The mutation that produced this re-render created a node under
        // this very picker: leave the filter empty (no `input` dispatched,
        // so the group-folding it drives stays at rest) and show the
        // "added" signal instead. A host without the T2 markup (the `p` is
        // simply absent) just skips the signal.
        var addedName = box.querySelector("[data-add-added-name]");
        var undo = box.querySelector("[data-add-undo]");
        var added = box.querySelector(".editor-add__added");
        if (addedName && undo && added) {
          addedName.textContent = leafName(createdPath);
          undo.dataset.remove = createdPath;
          added.hidden = false;
        }
      } else if (filter && saved.filter) {
        filter.value = saved.filter;
        filter.dispatchEvent(new Event("input", { bubbles: true }));
      }
      if (saved.focusFilter && filter) filter.focus();
      // The typeahead already folded the groups for a restored filter; an
      // empty filter (either because it never had one, or because `owns`
      // left it empty) returns the Extensions group to what it was before
      // the swap instead of the default collapsed state.
      if ((!filter || !filter.value) && saved.extOpen) {
        var extGroup = box.querySelector('details[data-add-group="extensions"]');
        if (extGroup) extGroup.setAttribute("open", "");
      }
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

  /* Closes `box` (a `details.editor-add--picker`) and returns focus to its
   * own toggle — used by the × button, Escape, and a click outside it. */
  function closePicker(box) {
    if (!box) return;
    box.removeAttribute("open");
    var summary = box.querySelector("summary");
    if (summary) summary.focus();
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
      // A non-empty filter unfolds whichever group still has a match;
      // cleared, it returns to the default state (Elements open,
      // Extensions collapsed).
      panel.querySelectorAll(".editor-add__group").forEach(function (group) {
        group.open = needle
          ? group.querySelector("[data-add-name]:not([hidden])") !== null
          : !group.hasAttribute("data-add-group");
      });
      // Typing means the user moved on from whatever was just added.
      var added = panel.querySelector(".editor-add__added");
      if (added) added.hidden = true;
    });

    // Closing the panel: the × button, and Escape while focus is inside it
    // — `stopPropagation` keeps it from reaching any other Escape handler a
    // host installs of its own (the Resources workspace closes its whole
    // edit surface on Escape, and this picker sits inside it). The native
    // `<details>` toggle on the summary still opens and closes it as
    // before.
    container.addEventListener("click", function (event) {
      var close = event.target.closest("[data-add-close]");
      if (close) closePicker(close.closest("details.editor-add--picker"));
    });
    container.addEventListener("keydown", function (event) {
      if (event.key !== "Escape") return;
      var box = event.target.closest("details.editor-add--picker[open]");
      if (!box) return;
      closePicker(box);
      event.preventDefault();
      event.stopPropagation();
    });

    // A click outside an open picker closes it too, without moving focus.
    // Registered on `document` once per container (its own guard, since
    // this runs even though the container-level guard above already keeps
    // `attach` itself from re-running).
    if (!container.dataset.hfsEditorAddOutside) {
      container.dataset.hfsEditorAddOutside = "1";
      document.addEventListener("click", function (event) {
        container.querySelectorAll("details.editor-add--picker[open]").forEach(function (box) {
          if (!box.contains(event.target)) box.removeAttribute("open");
        });
      });
    }
  }

  return {
    capturePickers: capturePickers,
    restorePickers: restorePickers,
    extensionUrl: extensionUrl,
    attach: attach,
    matches: matches,
    parentPath: parentPath,
    leafName: leafName,
  };
});
