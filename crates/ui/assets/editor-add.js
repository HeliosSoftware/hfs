/* Shared Guided editor additions: picker state and ownership, exact-path
 * focus and tree-only reveal, and transient row-local Undo. All three host
 * adapters use this module; the host container survives each server swap.
 * Markup and localized messages come from the shared Askama form partial.
 * UMD exports let Node test these helpers without a browser framework. */
(function (root, factory) {
  "use strict";

  var api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.HfsEditorAdd = api;
})(typeof window !== "undefined" ? window : null, function () {
  "use strict";

  /* Exact row identities, including the root and collection headings. */
  function rowByPath(container, path) {
    var rows = container.querySelectorAll(".editor-row[data-path]");
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
        focusFilter: filter === active,
        // Only a group the user unfolded on their own is worth restoring:
        // with a filter typed, the typeahead decides, and once the filter
        // is cleared the group goes back to folded (#1239).
        extOpen: !!(extGroup && extGroup.open) && !(filter && filter.value.trim()),
      });
    });
    return pickers;
  }

  function restorePickers(container, pickers, createdPath, operation) {
    (pickers || []).forEach(function (saved) {
      var row = rowByPath(container, saved.path);
      if (!row) return;
      var box = row.querySelector("details.editor-add--picker");
      if (!box) return;
      if (createdPath && operation && operation.pickerPath === saved.path) {
        box.removeAttribute("open");
        return;
      }
      box.setAttribute("open", "");
      var filter = box.querySelector(".editor-add__filter");
      if (filter && saved.filter) {
        filter.value = saved.filter;
        filter.dispatchEvent(new Event("input", { bubbles: true }));
      }
      if (saved.focusFilter && filter) filter.focus({ preventScroll: true });
      if ((!filter || !filter.value) && saved.extOpen) {
        var extGroup = box.querySelector('details[data-add-group="extensions"]');
        if (extGroup) extGroup.setAttribute("open", "");
      }
    });
  }

  function operationFrom(trigger) {
    var row = trigger && trigger.closest(".editor-row[data-path]");
    var box = trigger && trigger.closest("details.editor-add--picker");
    var owner = box && box.closest(".editor-row[data-path]");
    return {
      originPath: row ? row.dataset.path : "",
      pickerPath: owner ? owner.dataset.path : null,
    };
  }

  // Survives full-body and card-only swaps. Blur starts set before a click
  // can activate Undo, so pending state must be synchronous, before fetch.
  var requests = new WeakMap();
  function pendingCount(container) { return requests.get(container) || 0; }
  var mutationQueues = new WeakMap();
  var documentVersions = new WeakMap();
  function documentVersion(container) { return documentVersions.get(container) || 0; }
  function invalidateRefresh(container) {
    documentVersions.set(container, documentVersion(container) + 1);
  }

  // A structural request changes indexed paths. Until its projection arrives,
  // the displayed controls cannot safely submit another path-based operation.
  var projections = new WeakMap();
  function projectionBusy(container) {
    var state = projections.get(container);
    return !!(state && state.pending);
  }
  function lockProjection(container) {
    var state = projections.get(container);
    if (!state || !state.pending) return;
    var controls = container.querySelectorAll("[data-set], [data-add], [data-remove], [data-choose], [data-extension]");
    for (var i = 0; i < controls.length; i++) {
      if (!state.controls.has(controls[i])) state.controls.set(controls[i], controls[i].disabled);
      controls[i].disabled = true;
    }
    if (container.setAttribute) {
      container.setAttribute("data-editor-structural-pending", "");
      container.setAttribute("aria-busy", "true");
    }
  }
  function beginProjection(container) {
    var state = { pending: true, controls: new Map() };
    projections.set(container, state);
    state.finish = function () {
      if (!state.pending) return;
      state.pending = false;
      state.controls.forEach(function (disabled, control) { control.disabled = disabled; });
      state.controls.clear();
      if (container.removeAttribute) {
        container.removeAttribute("data-editor-structural-pending");
        container.removeAttribute("aria-busy");
      }
    };
    lockProjection(container);
    return state.finish;
  }
  function projectionSwapped(container, op) {
    var state = projections.get(container);
    // The final response must unlock before created-row focus is restored.
    if (state && op && op !== "set") state.finish();
    else lockProjection(container);
  }

  /* A blur/set and its following click/add depend on the same document.
   * Reserve their pending state immediately, but let the adapter read its
   * document only when work starts, after the previous swap has completed.
   * A failed prerequisite rejects the rest of that burst; a later explicit
   * retry starts a new burst rather than appending to a rejected tail. */
  function queueMutation(container, work, onFailure, op) {
    if (op && projectionBusy(container)) return Promise.resolve();
    var finishProjection = op && op !== "set" ? beginProjection(container) : function () {};
    var queue = mutationQueues.get(container);
    if (!queue || !queue.pending) {
      queue = { tail: Promise.resolve(), pending: 0 };
      mutationQueues.set(container, queue);
    }
    queue.pending++;
    invalidateRefresh(container);
    var finish = beginRequest(container);
    var started = false;
    var result = queue.tail.then(function () {
      started = true;
      return work();
    });
    queue.tail = result;
    return result.catch(function (error) {
      // A failed set also cancels an already reserved structural follower.
      // Release its projection before restoring the retained dirty field.
      if (started) projectionSwapped(container, "failed");
      finishProjection();
      if (started && onFailure) onFailure(error);
      throw error;
    }).finally(function () {
      queue.pending--;
      finishProjection();
      finish();
    });
  }

  function failedMutation(container, op, fields) {
    announce(container, statusText(container, "msgFailed"));
    var failure = container.querySelector("[data-editor-failure]");
    if (failure) failure.hidden = false;
    if (op !== "set") return;
    var controls = container.querySelectorAll("[data-set]");
    for (var i = 0; i < controls.length; i++) {
      if (controls[i].dataset.set === fields.path) {
        controls[i].setAttribute("aria-invalid", "true");
        controls[i].title = statusText(container, "msgFailed");
        var row = rowByPath(container, fields.path);
        var head = row && row.querySelector(".editor-row__head");
        if (failure && head) head.insertAdjacentElement("afterend", failure);
        // Keep the unsubmitted value and make the next explicit action retry
        // this field's blur before another dependent structural mutation.
        var active = typeof document !== "undefined" ? document.activeElement : null;
        if (!active || active === document.body || (container.contains(active) && !active.matches("[data-set]"))) {
          controls[i].focus({ preventScroll: true });
        }
        break;
      }
    }
  }

  function statusText(container, key) {
    var status = container.querySelector("[data-add-status]");
    return status ? status.dataset[key] || "" : "";
  }

  function announce(container, text) {
    var status = container.querySelector("[data-add-status]");
    if (!status) return;
    status.textContent = "";
    var generation = (status.hfsAnnouncement || 0) + 1;
    status.hfsAnnouncement = generation;
    setTimeout(function () {
      if (container.contains(status) && status.hfsAnnouncement === generation) status.textContent = text;
    }, 50);
  }

  function updateUndoPending(container) {
    var undo = container.querySelector("[data-add-undo]");
    if (!undo) return;
    var pending = pendingCount(container) > 0;
    undo.disabled = pending;
    if (pending) {
      undo.setAttribute("aria-busy", "true");
      undo.title = statusText(container, "msgPending");
    } else {
      undo.removeAttribute("aria-busy");
      undo.removeAttribute("title");
    }
  }

  function beginRequest(container) {
    requests.set(container, pendingCount(container) + 1);
    updateUndoPending(container);
    var note = container.querySelector("[data-add-undo-note]");
    if (note && !note.hidden) announce(container, statusText(container, "msgPending"));
    var ended = false;
    return function () {
      if (ended) return;
      ended = true;
      requests.set(container, Math.max(0, pendingCount(container) - 1));
      updateUndoPending(container);
    };
  }

  function canonicalDocument(text) {
    try { return JSON.stringify(JSON.parse(text)); } catch (invalid) { return null; }
  }

  // Remove-created only, not history. Also reject a changed live document
  // before the raw/CodeMirror debounced replacement has reached the server.
  function undoOperation(container, button, documentText) {
    var note = button.closest("[data-add-undo-note]");
    var saved = note && note.hfsUndo;
    if (pendingCount(container) || button.disabled) {
      announce(container, statusText(container, "msgPending"));
      return null;
    }
    if (!saved || !container.contains(button) || saved.document !== canonicalDocument(documentText)) {
      if (note) note.hidden = true;
      announce(container, statusText(container, "msgUnavailable"));
      return null;
    }
    return { undo: true, originPath: saved.originPath, createdPath: saved.createdPath };
  }

  // Keep scrolling inside the tree; its viewport may itself extend below
  // the window, so reveal in their visible intersection. Already-visible
  // controls don't move. Fitting complex rows include their Undo/Add actions;
  // oversized rows reveal their heading instead of trying to fit the subtree.
  function revealInTree(tree, target, oversizedFallback) {
    if (!tree || !target) return;
    var pane = tree.getBoundingClientRect();
    var rect = target.getBoundingClientRect();
    var viewport = typeof window !== "undefined" ? window.innerHeight : Infinity;
    var top = Math.max(0, pane.top + tree.clientTop);
    var bottom = Math.min(viewport, pane.top + tree.clientTop + tree.clientHeight);
    if (bottom <= top) return;
    if (oversizedFallback && rect.bottom - rect.top > bottom - top) {
      target = oversizedFallback;
      rect = target.getBoundingClientRect();
    }
    if ((rect.top < top || rect.bottom > bottom) && pane.top + tree.clientTop + tree.clientHeight > viewport && viewport - pane.top >= 64) {
      // Without this bound the last input can hit maximum scroll while still
      // being below the window: the tree's own bottom is offscreen too.
      tree.style.maxHeight = Math.floor(viewport - pane.top) + "px";
      pane = tree.getBoundingClientRect();
      rect = target.getBoundingClientRect();
      top = Math.max(0, pane.top + tree.clientTop);
      bottom = Math.min(viewport, pane.top + tree.clientTop + tree.clientHeight);
    }
    if (rect.top < top) tree.scrollTop += rect.top - top;
    else if (rect.bottom > bottom) tree.scrollTop += Math.min(rect.bottom - bottom, rect.top - top);
  }

  function revealCreated(container, createdPath, operation) {
    if (!createdPath) return false;
    var row = rowByPath(container, createdPath);
    if (!row) return false;
    // Only a primitive's own value control edits the created node. A complex
    // row's data-choose selector adds a child arm from inside its picker;
    // matching that parent's path does not make it the created node's input.
    var controls = row.querySelectorAll("[data-set]");
    var target = null;
    for (var i = 0; i < controls.length; i++) {
      if (controls[i].dataset.set === createdPath) { target = controls[i]; break; }
    }
    if (operation && operation.pickerPath !== null) {
      var owner = rowByPath(container, operation.pickerPath);
      var picker = owner && owner.querySelector("details.editor-add--picker");
      if (picker) picker.removeAttribute("open");
    }
    if (!target) target = row;
    target.focus({ preventScroll: true });
    if (target.select) target.select();

    var note = container.querySelector("[data-add-undo-note]");
    var undo = note && note.querySelector("[data-add-undo]");
    var doc = container.querySelector("#editor-doc");
    if (operation && note && undo && doc) {
      note.hfsUndo = {
        originPath: operation.originPath,
        createdPath: createdPath,
        document: canonicalDocument(doc.value),
      };
      undo.dataset.remove = createdPath;
      var head = row.querySelector(".editor-row__head");
      if (head) head.insertAdjacentElement("afterend", note);
      else row.appendChild(note);
      note.hidden = false;
      updateUndoPending(container);
      announce(container, leafName(createdPath) + " " + statusText(container, "msgAdded"));
    }
    // Place the action first, so the final scroll range includes its height.
    revealInTree(container.querySelector(".editor-tree"), target, target === row ? row.querySelector(".editor-row__head") : null);
    return true;
  }

  function restoreUndoFocus(container, operation) {
    if (!operation || !operation.undo) return false;
    var collectionPath = operation.createdPath.replace(TRAILING_INDEX, "");
    var collection = collectionPath !== operation.createdPath && rowByPath(container, collectionPath);
    var origin = rowByPath(container, operation.originPath);
    var parent = rowByPath(container, parentPath(operation.createdPath));
    var row = collection || origin || parent || rowByPath(container, "");
    if (!row) return false;
    // A sliced collection's Add is a disclosure; options inside it are hidden.
    var picker = row.querySelector("details.editor-add--picker");
    var target = picker ? picker.querySelector("summary") : row.querySelector("[data-collection-add]");
    if (!target && parent && parent !== row) {
      picker = parent.querySelector("details.editor-add--picker");
      target = picker && picker.querySelector("summary");
    }
    if (!target) target = row;
    target.focus({ preventScroll: true });
    revealInTree(container.querySelector(".editor-tree"), target);
    return true;
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

  function activePrimitiveForAction(container, event) {
    var action = event.target.closest("[data-add], [data-remove], [data-extension]");
    var active = typeof document !== "undefined" ? document.activeElement : null;
    return action && !action.disabled && active && container.contains(active) && active.matches("[data-set]") ? active : null;
  }

  function holdDirtyPointer(container, event) {
    if (event.button !== 0 || event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
    var input = activePrimitiveForAction(container, event);
    // Defer this blur until click. A fast set response between pointerdown
    // and pointerup would otherwise replace the very button being pressed.
    if (input && input.value !== input.defaultValue) event.preventDefault();
  }

  function flushDirtyPrimitive(container, event) {
    var input = activePrimitiveForAction(container, event);
    // Capture runs before the adapter's bubbling action handler. Blur and
    // the action reserve their queue positions within the same click turn.
    if (input) input.blur();
  }

  function attach(container) {
    if (!container || container.dataset.hfsEditorAdd === "1") return;
    container.dataset.hfsEditorAdd = "1";
    container.addEventListener("pointerdown", function (event) { holdDirtyPointer(container, event); }, true);
    container.addEventListener("click", function (event) { flushDirtyPrimitive(container, event); }, true);
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
    holdDirtyPointer: holdDirtyPointer,
    flushDirtyPrimitive: flushDirtyPrimitive,
    matches: matches,
    parentPath: parentPath,
    leafName: leafName,
    rowByPath: rowByPath,
    operationFrom: operationFrom,
    beginRequest: beginRequest,
    undoOperation: undoOperation,
    revealCreated: revealCreated,
    revealInTree: revealInTree,
    restoreUndoFocus: restoreUndoFocus,
    canonicalDocument: canonicalDocument,
    queueMutation: queueMutation,
    projectionBusy: projectionBusy,
    projectionSwapped: projectionSwapped,
    documentVersion: documentVersion,
    invalidateRefresh: invalidateRefresh,
    failedMutation: failedMutation,
  };
});
