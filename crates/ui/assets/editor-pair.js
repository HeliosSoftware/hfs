/*
 * Shared host between a CodeMirror/textarea JSON editor and the guided-form
 * card beside it (#843), extracted out of `vd-editor.js` for #840 so View
 * Definitions and the Library "Details" section (#840) share one
 * implementation instead of two copies of the same sync/highlight logic.
 *
 * Repartition of responsibilities across the editor family (#840):
 *   - `code-editor.js`   mounts CodeMirror over a `<textarea>`: wrapper,
 *                         aria-label, tabindex, Tab-not-captured, and the
 *                         shared JSON token-color preset (`jsonHighlight()`).
 *   - `editor-form.js`   drives the guided-form card's own interactions
 *                         (`[data-add]`/`[data-remove]`/`[data-set]`/... ->
 *                         `POST /ui/editor/render`) against a caller-supplied
 *                         host, including any extra `fields` that host adds
 *                         to every request.
 *   - `editor-pair.js`   (this file) is that host: keeps the editor and the
 *                         guided-form card showing the same document, in
 *                         both directions, plus the validity chip and the
 *                         row<->editor cross-highlight.
 *   - `vd-editor.js` (and #840's `sql-library-details.js`) own only what is
 *                         specific to their own page - the ViewDefinition's
 *                         FHIRPath language/highlighting/lint for the
 *                         former, nothing but the mount call for the latter
 *                         - and hand this file the textarea, the mounted
 *                         `EditorView` (or nothing, textarea fallback), and
 *                         the guided-form grid.
 *
 * `window.HfsEditorPair.mount(options)`:
 *   - `textarea` (required) - the `<textarea>` that is the document's
 *     source of truth.
 *   - `view` (optional) - the `EditorView` `code-editor.js` mounted over
 *     that textarea; absent (bundle never loaded, or the mount failed) runs
 *     the same sync against the plain textarea instead, no CodeMirror
 *     concept involved.
 *   - `grid` (required) - the container that already holds the hidden
 *     `#editor-form` state, the guided-form `section.editor-form` card, and
 *     this render's `<datalist>`s (`editor::build_form_pane`'s own markup -
 *     what `editor-form.js` calls `root`).
 *   - `fields` (optional) - extra `{name: value}` pairs added to every
 *     request this pairing sends through `editor-form.js` (mutations and
 *     the JSON -> form `refresh` below), e.g. `{ hidden: "content", legend:
 *     "sql-library" }` for a Library's Details document (#840).
 *   - `invalidJsonMessage` (optional) - the text the validity chip
 *     (`.editor-validity`) switches to while the editor's text does not
 *     parse as JSON at all. Defaults to `grid`'s initial
 *     `.editor-validity`'s own `data-msg-invalid-json` attribute, read once
 *     at mount - a round trip through `editor-form.js` always swaps that
 *     element for one `editor::build_form_pane` renders with `needs_js`
 *     false (no attribute at all, #843), so reading it fresh on every
 *     invalid keystroke instead would leave the chip's text stuck on
 *     whatever it last showed. Pass this explicitly for a host whose
 *     initial markup never carries the attribute either.
 *
 * Returns `{ formApi, host }` - `editor-form.js`'s own `attach()` return
 * value, plus the host object built here - or does nothing (returns
 * `undefined`) when `textarea`, `grid`, or `window.HfsEditorForm` is
 * missing, the same silent-degradation contract every script in this family
 * follows.
 *
 * The sync, in both directions:
 *   - a form-driven change lands in the editor as one minimal transaction
 *     (common-prefix/common-suffix diff, `minimalChange` below) tagged with
 *     a local `FormOriginEffect`, so it is undoable, does not move the
 *     scroll or the caret, and - because `code-editor.js`'s own listener
 *     fires on any `docChanged` update regardless of cause - still updates
 *     the hidden textarea and fires `input` exactly like a manual edit; in
 *     textarea mode, the same change simply assigns `textarea.value` and
 *     fires `input` itself.
 *   - an editor change that is NOT that echo is parsed 600ms after the last
 *     keystroke and, if it parses and its canonical form actually changed,
 *     re-requests the guided-form panel alone - invalid JSON never reaches
 *     the server; the panel keeps its rows and the validity chip
 *     (`.editor-validity`) switches to a short client-side "invalid JSON"
 *     state instead, restored the moment the text parses again.
 *
 * Cross-highlight (only with `view`, since it needs the editor's syntax
 * tree): hovering or focusing a guided-form row paints the lines its node
 * occupies in the editor (a CodeMirror `StateField` of line decorations,
 * `cm-line--hit` in `app.css`) and reveals them inside the editor's own
 * viewport; moving the cursor in the editor marks the row for the node it
 * now sits in (or its nearest ancestor with a row), 120ms after it settles,
 * and reveals it inside `.editor-tree`. Both directions resolve a node by
 * walking the browser's own CodeMirror syntax tree - a row's dotted path
 * (`select.0.column.0.path`) down to its node, or a cursor position up
 * through its ancestors back to a dotted path - never by re-parsing the
 * text by hand. Only one direction is painted at a time, and reveal stays
 * inside each pane's own scroll container, never the page's. This file adds
 * the CodeMirror extensions the cross-highlight and the sync need
 * (an update listener, a decoration `StateField`) onto the `view` already
 * mounted, via `StateEffect.appendConfig` - the caller never has to pass
 * anything pair-specific into `HfsCodeEditor.mount`.
 *
 * Without a `view`, without JS, or if anything above throws during
 * construction, this file does nothing (or backs out cleanly) and the
 * caller's own textarea fallback keeps working exactly as it would without
 * this file at all.
 *
 * `minimalChange` is exported for its own unit test
 * (`crates/ui/e2e/unit/editor-pair.test.cjs`) via the same UMD-ish shape
 * `assets/combobox.js` uses; `mount` only ever runs when called from a real
 * page, so requiring this file under Node defines the functions and does
 * nothing else.
 */
(function (root, factory) {
  "use strict";

  var api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.HfsEditorPair = api;
})(typeof window !== "undefined" ? window : null, function () {
  "use strict";

  /* ---- minimal single-range diff (#843) ----------------------------------
   *
   * The common-prefix/common-suffix diff between two strings, as the one
   * `{from, to, insert}` CodeMirror change that turns `oldText` into
   * `newText` - the smallest edit that does the job, not a general diff: a
   * single contiguous replaced range is exactly what preserves every
   * position outside it (so CodeMirror's own position mapping leaves the
   * caret, the scroll offset, and anything else anchored elsewhere in the
   * document undisturbed) and folds into one undo step. `null` when the two
   * texts are identical - nothing changed, so `setDoc` below dispatches no
   * transaction at all for that case.
   */
  function minimalChange(oldText, newText) {
    if (oldText === newText) return null;

    var oldLen = oldText.length;
    var newLen = newText.length;
    var max = Math.min(oldLen, newLen);

    var prefix = 0;
    while (prefix < max && oldText.charCodeAt(prefix) === newText.charCodeAt(prefix)) {
      prefix++;
    }

    var maxSuffix = max - prefix;
    var suffix = 0;
    while (
      suffix < maxSuffix &&
      oldText.charCodeAt(oldLen - 1 - suffix) === newText.charCodeAt(newLen - 1 - suffix)
    ) {
      suffix++;
    }

    return {
      from: prefix,
      to: oldLen - suffix,
      insert: newText.slice(prefix, newLen - suffix),
    };
  }

  /* ---- JSON syntax-tree walking, dotted-path form (#843, cross-highlight)
   *
   * The row<->editor link keys rows on the same dotted path
   * `editor::path_to_string` uses (`select.0.column.0.path`), never an RFC
   * 6901 pointer - the tree walk itself (`resolveBySegments`) is otherwise
   * identical either way, an object key by name or an array index
   * numerically, so it is shared here even though `vd-editor.js` keeps its
   * own small pointer-based copy for its lint diagnostics: two independent
   * features that both happen to walk a JSON syntax tree, not one that
   * belongs to a shared JSON-tree module of its own. */

  function propertyKeyText(propertyNameNode, doc) {
    var raw = doc.sliceString(propertyNameNode.from, propertyNameNode.to);
    return raw.length >= 2 && raw.charAt(0) === '"' && raw.charAt(raw.length - 1) === '"'
      ? raw.slice(1, -1)
      : raw;
  }

  /* The Property node inside `objectNode` whose PropertyName reads `key`. */
  function findProperty(objectNode, key, doc) {
    for (var child = objectNode.firstChild; child; child = child.nextSibling) {
      if (child.name !== "Property") continue;
      var nameNode = child.firstChild;
      if (nameNode && nameNode.name === "PropertyName" && propertyKeyText(nameNode, doc) === key) {
        return child;
      }
    }
    return null;
  }

  /* A Property's value is its last child (PropertyName and ":" come first). */
  function propertyValue(propertyNode) {
    var last = propertyNode.lastChild;
    return last && last.name !== ":" ? last : null;
  }

  function arrayItemAt(arrayNode, index) {
    var i = 0;
    for (var child = arrayNode.firstChild; child; child = child.nextSibling) {
      if (child.name === "[" || child.name === "]" || child.name === ",") continue;
      if (i === index) return child;
      i++;
    }
    return null;
  }

  /* The inverse of `arrayItemAt`: the index of `target` among `arrayNode`'s
   * own items, or -1 if `target` is not one of them. Compared by range
   * rather than reference - a node handed back by `tree.resolveInner` is
   * not guaranteed to be the exact object identity `firstChild`/
   * `nextSibling` walk past, only the same `{from, to}` span. */
  function arrayIndexOfChild(arrayNode, target) {
    var i = 0;
    for (var child = arrayNode.firstChild; child; child = child.nextSibling) {
      if (child.name === "[" || child.name === "]" || child.name === ",") continue;
      if (child.from === target.from && child.to === target.to) return i;
      i++;
    }
    return -1;
  }

  /* Walks the JSON syntax tree from its root value, following `segments`
   * (an object key by name, an array index numerically), and returns the
   * node at that position - or null if the path does not resolve against
   * this document's actual shape (a half-typed edit, most often). */
  function resolveBySegments(tree, doc, segments) {
    var node = tree.topNode.getChild("Object") || tree.topNode.firstChild;
    if (!node) return null;
    for (var i = 0; i < segments.length; i++) {
      var segment = segments[i];
      if (node.name === "Object") {
        var property = findProperty(node, segment, doc);
        if (!property) return null;
        node = propertyValue(property);
      } else if (node.name === "Array") {
        if (!/^\d+$/.test(segment)) return null;
        node = arrayItemAt(node, Number(segment));
      } else {
        return null;
      }
      if (!node) return null;
    }
    return node;
  }

  function dottedSegments(path) {
    return path === "" ? [] : path.split(".");
  }

  function resolveByDottedPath(tree, doc, path) {
    return resolveBySegments(tree, doc, dottedSegments(path));
  }

  /* ---- mount --------------------------------------------------------------- */

  function mount(options) {
    options = options || {};
    var textarea = options.textarea;
    var grid = options.grid;
    var EditorForm = window.HfsEditorForm;
    if (!textarea || !grid || !EditorForm) return;

    var view = options.view || null;
    var CM = window.HfsCodeMirror;

    var formApi = { refresh: function () {} };
    var lastSynced = null;
    var syncTimer = null;
    var invalidChip = null;
    var FormOriginEffect = null;

    /* The "Invalid JSON" chip text, captured once here rather than read
     * fresh off the chip element every time `markChipInvalid` needs it: the
     * card `editor-form.js` swaps in after a mutation round trip is always
     * the `needs_js`-less render of `editor-form-pane.html` (every renderer
     * of this partial except a page's own inline first paint leaves
     * `needs_js` false, since the standalone Resource Editor - the only
     * other consumer - fills `#editor-body` from its own client-side fetch
     * with no such gate to begin with) - so its own `data-msg-invalid-json`
     * is only ever present on the very first card this pairing sees, before
     * any round trip has replaced it. `options.invalidJsonMessage` lets a
     * caller override or supply it outright (e.g. a host whose initial
     * markup does not carry the attribute at all); reading the mounted
     * chip is the fallback, not the primary source, so a later mutation
     * response missing the attribute never blanks this out. */
    var invalidJsonMessage = options.invalidJsonMessage || "";
    if (!invalidJsonMessage) {
      var initialChip = grid.querySelector(".editor-validity");
      if (initialChip) invalidJsonMessage = initialChip.dataset.msgInvalidJson || "";
    }

    if (view && CM) {
      /* ---- the transaction-origin marker ----------------------------------
       *
       * A `StateEffect` carried on the one transaction `host.setDoc` (below)
       * dispatches for a form-driven change - not a `StateField`, since
       * nothing needs to persist this past the transaction that carries it.
       * `handleCmUpdate`'s sync listener checks for it to skip its own
       * form's echo, without needing to compare document text at all for
       * that case.
       */
      FormOriginEffect = CM.StateEffect.define();

      /* ---- row <-> editor cross-highlight ----------------------------------
       *
       * `.editor-row--hit` already exists (the Resource Editor's own
       * `editor-sync.js` set it long before this pairing did); the JSON side
       * has no equivalent to reach for, because a CodeMirror `EditorView`
       * has no server-rendered `.json-line[data-jpath]` line the way the
       * Resource Editor's raw JSON view does. A line decoration is CM6's own
       * notion of "paint this line": a `StateField` holding the current
       * decoration set, replaced wholesale on every `HighlightEffect` and
       * reset to none the moment the document itself changes - a line range
       * computed against the old text is meaningless against the new one,
       * and recomputing it is exactly what the next hover or cursor move
       * already does on its own.
       *
       * `hitKey` below is the single source of truth for which one of the
       * two directions (if either) is currently painted, the same
       * mutually-exclusive "one hit at a time" model `editor-sync.js` uses
       * for its own `__hitKey` - hovering a row clears a prior
       * cursor-driven row mark before painting the editor, and vice versa.
       */
      var HighlightEffect = CM.StateEffect.define();
      var lineHighlightMark = CM.Decoration.line({ class: "cm-line--hit" });
      var highlightField = CM.StateField.define({
        create: function () {
          return CM.Decoration.none;
        },
        update: function (decorations, tr) {
          if (tr.docChanged) return CM.Decoration.none;
          for (var i = 0; i < tr.effects.length; i++) {
            if (tr.effects[i].is(HighlightEffect)) return tr.effects[i].value;
          }
          return decorations;
        },
        provide: function (field) {
          return CM.EditorView.decorations.from(field);
        },
      });

      var hitKey = null;
      var selectionTimer = null;

      /* Row -> editor: the line range a row's node occupies, from the line
       * of its own key (its enclosing Property, not just its value - a
       * multi-line object's key sits on the line above its own `{`) through
       * the line its value closes on. An array item has no key of its own,
       * so its node's own `from` (the item's opening bracket/quote/digit)
       * is where the range starts instead. `null` when `path` does not
       * resolve against the document's current shape (a half-typed edit,
       * most often), so the caller paints nothing rather than a stale or
       * wrong range. */
      var nodeLineRange = function (path) {
        var tree = CM.syntaxTree(view.state);
        var node = resolveByDottedPath(tree, view.state.doc, path);
        if (!node) return null;
        var property = node.parent && node.parent.name === "Property" ? node.parent : null;
        return { from: property ? property.from : node.from, to: node.to };
      };

      /* Editor -> row: the dotted path of the node the cursor sits in,
       * climbing the syntax tree from the innermost node at the cursor
       * (`side: -1` so a cursor right after a closing quote still resolves
       * to the string it just left, not whatever follows). Landing inside a
       * Property's own key counts as that property, by construction: a leaf
       * inside `PropertyName`, or `resolveInner` handing back the
       * `Property` node itself outright (a cursor sitting in the gap
       * between key and value resolves no deeper than that), both hit the
       * `node.name === "Property"` branch and capture that property's key
       * before climbing past it. */
      var nodePathAtCursor = function () {
        var tree = CM.syntaxTree(view.state);
        var doc = view.state.doc;
        var node = tree.resolveInner(view.state.selection.main.head, -1);
        var segments = [];
        while (node) {
          if (node.name === "Property") {
            var nameNode = node.firstChild;
            if (nameNode && nameNode.name === "PropertyName") {
              segments.unshift(propertyKeyText(nameNode, doc));
            }
            node = node.parent;
            continue;
          }
          var parent = node.parent;
          if (!parent) break;
          if (parent.name === "Array") {
            var index = arrayIndexOfChild(parent, node);
            if (index !== -1) segments.unshift(String(index));
          }
          node = parent;
        }
        return segments.join(".");
      };

      /* The row for `path`, or - failing that - its nearest ancestor row
       * (stripping one dotted segment at a time) down to and including the
       * root row itself (`path: ""`, always present: `build_form_pane`
       * gives the document's own top-level object a row). */
      var rowForPathOrAncestor = function (path) {
        for (;;) {
          var rows = grid.querySelectorAll(".editor-row[data-path]");
          for (var i = 0; i < rows.length; i++) {
            if (rows[i].dataset.path === path) return rows[i];
          }
          if (path === "") return null;
          var cut = path.lastIndexOf(".");
          path = cut === -1 ? "" : path.slice(0, cut);
        }
      };

      /* Container-only reveal (`.editor-tree`'s own scroll, never the
       * page's) - the same shape as `editor-sync.js`'s own `reveal`, kept
       * as its own small copy here rather than imported: that file stays
       * untouched, and importing one six-line helper across a page
       * boundary is not worth the coupling. */
      var revealRow = function (row) {
        var tree = grid.querySelector(".editor-tree");
        if (!tree || !row || tree.scrollHeight <= tree.clientHeight) return;
        var delta = row.getBoundingClientRect().top - tree.getBoundingClientRect().top;
        var top = tree.scrollTop + delta;
        if (top < tree.scrollTop || top > tree.scrollTop + tree.clientHeight - 32) {
          tree.scrollTop = Math.max(0, top - tree.clientHeight / 3);
        }
      };

      /* Clears whichever of the two directions is currently painted - a
       * no-op, no dispatch included, once nothing is. */
      var clearHighlight = function () {
        if (hitKey === null) return;
        hitKey = null;
        clearTimeout(selectionTimer);
        view.dispatch({ effects: HighlightEffect.of(CM.Decoration.none) });
        grid.querySelectorAll(".editor-row--hit").forEach(function (row) {
          row.classList.remove("editor-row--hit");
        });
      };

      /* Row -> editor: paints every line `path`'s node spans and brings the
       * first one into the editor's own viewport - never the page's, since
       * `EditorView.scrollIntoView` only ever moves `.cm-scroller`. */
      var highlightPathInEditor = function (path) {
        var range = nodeLineRange(path);
        if (!range) return;
        var doc = view.state.doc;
        var firstLine = doc.lineAt(range.from).number;
        var lastLine = doc.lineAt(range.to).number;
        var lines = [];
        for (var n = firstLine; n <= lastLine; n++) {
          lines.push(lineHighlightMark.range(doc.line(n).from));
        }
        hitKey = "r:" + path;
        view.dispatch({
          effects: [
            HighlightEffect.of(CM.Decoration.set(lines)),
            CM.EditorView.scrollIntoView(doc.line(firstLine).from, { y: "nearest" }),
          ],
        });
      };

      var handleRowEnter = function (event) {
        var row = event.target.closest ? event.target.closest(".editor-row[data-path]") : null;
        if (!row || !grid.contains(row)) return;
        var key = "r:" + row.dataset.path;
        if (hitKey === key) return;
        clearHighlight();
        highlightPathInEditor(row.dataset.path);
      };

      var handleRowLeave = function (event) {
        var row = event.target.closest ? event.target.closest(".editor-row[data-path]") : null;
        if (!row) return;
        var to = event.relatedTarget;
        if (to && grid.contains(to) && to.closest && to.closest(".editor-row[data-path]")) return;
        clearHighlight();
      };

      /* Editor -> row: 120ms after the cursor last moved (selection change
       * or a keystroke alike - both reach here through the same
       * `updateListener`, appended below), so a fast typist or an
       * in-progress selection drag does not churn the form pane's DOM on
       * every intermediate position. */
      var handleSelectionUpdate = function () {
        clearTimeout(selectionTimer);
        selectionTimer = setTimeout(function () {
          var path;
          try {
            path = nodePathAtCursor();
          } catch (unresolved) {
            path = null;
          }
          var row = path === null ? null : rowForPathOrAncestor(path);
          if (!row) {
            clearHighlight();
            return;
          }
          var key = "c:" + row.dataset.path;
          if (hitKey === key) return;
          clearHighlight();
          hitKey = key;
          row.classList.add("editor-row--hit");
          revealRow(row);
        }, 120);
      };

      // The page hands `HfsCodeEditor.mount` nothing pair-specific: this
      // pairing adds its own extensions onto the already-mounted `view`
      // instead, via `StateEffect.appendConfig` (a CM6 reconfiguration
      // effect - `handleCmUpdate` is declared further down this same
      // function, safe to reference here because function declarations,
      // unlike `var`, hoist their implementation too, and this facet is not
      // actually invoked until a real edit, long after `mount()` has
      // finished assigning everything it closes over).
      view.dispatch({
        effects: CM.StateEffect.appendConfig.of([
          highlightField,
          CM.EditorView.updateListener.of(handleCmUpdate),
          CM.EditorView.updateListener.of(function (update) {
            if (update.selectionSet || update.docChanged) handleSelectionUpdate();
          }),
        ]),
      });

      // Hovering or focusing a row lights its node in the editor, and
      // moving the cursor in the editor lights the row back; `grid` (not
      // `document`) is the delegation root, since a pairing has no
      // standalone-editor or Resources-modal sibling to share a
      // document-level listener with the way `editor-sync.js` does.
      grid.addEventListener("mouseover", handleRowEnter);
      grid.addEventListener("focusin", handleRowEnter);
      grid.addEventListener("mouseout", handleRowLeave);
      grid.addEventListener("focusout", handleRowLeave);
    }

    /* ---- the host contract -------------------------------------------
     *
     * CodeMirror mounted: the `EditorView` the caller handed in. Not
     * mounted (no bundle, no helper, a construction error) - the plain
     * `<textarea>` underneath it: `getDoc` reads its value, `setDoc` writes
     * it and fires `input` for the same live-preview wiring a manual edit
     * already triggers.
     */
    var host = view
      ? {
          getDoc: function () {
            return view.state.doc.toString();
          },
          setDoc: function (text) {
            var change = minimalChange(view.state.doc.toString(), text);
            if (change) {
              view.dispatch({
                changes: { from: change.from, to: change.to, insert: change.insert },
                effects: FormOriginEffect.of(true),
              });
            }
            rememberSynced(text);
          },
        }
      : {
          getDoc: function () {
            return textarea.value;
          },
          setDoc: function (text) {
            if (text !== textarea.value) {
              textarea.value = text;
              textarea.dispatchEvent(new Event("input", { bubbles: true }));
            }
            rememberSynced(text);
          },
        };
    if (options.fields) host.fields = options.fields;

    formApi = EditorForm.attach(grid, host);

    if (!view) {
      // No CodeMirror transaction to tag here, so no origin marker - the
      // textarea's own `input` event is the only signal, and `setDoc`
      // (above) fires it too on every form-driven change. The
      // canonical-form comparison below (`lastSynced`, updated by
      // `rememberSynced` before `setDoc` even returns) is what keeps that
      // self-triggered `input` from re-requesting the panel it just came
      // from.
      textarea.addEventListener("input", function () {
        scheduleSync(textarea.value);
      });
    }

    /* `text` is always `#editor-pretty`'s own value here - the server's
     * pretty-printed serialization of the document it just validated as
     * JSON, handed to `host.setDoc` by `editor-form.js` after every
     * mutation. Always parses; the try/catch is cheaper than trusting that
     * invariant blindly across a future change on either side. */
    function rememberSynced(text) {
      try {
        lastSynced = JSON.stringify(JSON.parse(text));
      } catch (alwaysValid) {
        /* see above */
      }
    }

    function chip() {
      return grid.querySelector(".editor-validity");
    }

    /* The validity chip switches to a short, purely client-side "invalid
     * JSON" state - no round trip, no touching the rows underneath. The
     * chip's own prior text/state is cached on first use so a later fix
     * that turns out to match `lastSynced` exactly (no refresh call, see
     * `scheduleSync`) still has something to restore it to. The message
     * itself comes from `invalidJsonMessage` (captured once at mount, see
     * above) rather than the *current* chip's own `data-msg-invalid-json` -
     * a round trip through `editor-form.js` swaps that element out for one
     * that never carries the attribute, so reading it fresh here would
     * leave the chip showing whatever text it already had instead of
     * switching to "Invalid JSON" at all. The dataset read stays as a
     * fallback for a host that never captured one (an empty
     * `invalidJsonMessage`, e.g. `options.invalidJsonMessage` omitted on an
     * initial card that itself carried no attribute either). */
    function markChipInvalid() {
      var el = chip();
      if (!el) return;
      if (!invalidChip) {
        invalidChip = {
          text: el.textContent,
          ok: el.classList.contains("editor-validity--ok"),
        };
      }
      el.classList.remove("editor-validity--ok");
      el.textContent = invalidJsonMessage || el.dataset.msgInvalidJson || el.textContent;
    }

    function clearChipInvalid() {
      if (!invalidChip) return;
      var el = chip();
      if (el) {
        el.textContent = invalidChip.text;
        if (invalidChip.ok) el.classList.add("editor-validity--ok");
      }
      invalidChip = null;
    }

    /* 600ms after the last keystroke, for a change the form did not itself
     * just cause. Invalid JSON never reaches the server - only the chip
     * changes, client-side, until the text parses again; a parseable text
     * whose canonical form did not actually move is likewise never sent -
     * this comparison is also the only guard the plain-textarea host has,
     * having no transaction to tag an origin onto. */
    function scheduleSync(text) {
      clearTimeout(syncTimer);
      syncTimer = setTimeout(function () {
        var parsed;
        try {
          parsed = JSON.parse(text);
        } catch (invalid) {
          markChipInvalid();
          return;
        }
        clearChipInvalid();
        var canonical = JSON.stringify(parsed);
        if (canonical === lastSynced) return;
        lastSynced = canonical;
        formApi.refresh(text);
      }, 600);
    }

    function handleCmUpdate(update) {
      if (!update.docChanged) return;
      var formOriginated = update.transactions.some(function (tr) {
        return tr.effects.some(function (effect) {
          return effect.is(FormOriginEffect);
        });
      });
      if (formOriginated) return;
      scheduleSync(update.state.doc.toString());
    }

    return { formApi: formApi, host: host };
  }

  return { minimalChange: minimalChange, mount: mount };
});
