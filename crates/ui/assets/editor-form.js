/*
 * Shared guided-form loop (#843), extracted from the standalone editor's
 * original (`editor.js`, still its own copy — migrating it, and the
 * Resources modal's own copy, onto this helper is a follow-up): every
 * interaction the guided form offers — `[data-add]`/`[data-remove]`/
 * `[data-extension]` clicks, `[data-choose]` changes, a settled `[data-set]`
 * blur, live `$expand` on a bound field — turned into one round trip to
 * `/ui/editor/render` (`pane=form`, #843) and a DOM swap, focus/caret/
 * picker/scroll preserved across it (#547) — without knowing which page it
 * runs on or what the other half of the document is. The add-picker's own
 * state across that swap, filter typeahead, and extension-URL read are
 * `window.HfsEditorAdd`'s job (#1239), shared with `editor.js` and
 * `resources.js` — this file calls it rather than keeping its own copy.
 *
 * `window.HfsEditorForm.attach(root, host)` wires all of that inside `root`
 * — an element that already contains the hidden `#editor-form` state, the
 * guided-form `section.editor-form` card, and this document's `<datalist>`s,
 * normally server-rendered on first paint (`editor::build_form_pane`) — and
 * drives `host` as the document's other half:
 *
 *   host.getDoc()      -> string, the document `host` holds right now.
 *   host.setDoc(text)  -> `text` lands back in `host` (a CodeMirror doc, a
 *                         plain textarea, ...) however `host` sees fit.
 *   host.renderUrl     -> optional override of "/ui/editor/render".
 *   host.fields        -> optional `{name: value}` pairs added to every
 *                         request this loop sends (mutations and `refresh`
 *                         alike) - e.g. `hidden`/`legend` for a document
 *                         that hides some of its own paths from the panel
 *                         (#840). Absent, behavior is unchanged.
 *
 * Returns `{ refresh(text) }`: re-renders `root` for `text` with no
 * mutation (`op=""`), for a caller driving JSON -> form sync from its own
 * side (`vd-editor.js`) — unlike every mutation `attach` wires up itself, a
 * `refresh` never calls `host.setDoc`, since the text it renders came from
 * `host` in the first place.
 */
(function () {
  "use strict";

  var DEFAULT_RENDER_URL = "/ui/editor/render";

  function attach(root, host) {
    if (!root || !host || !window.fetch) return { refresh: function () {} };
    var renderUrl = host.renderUrl || DEFAULT_RENDER_URL;
    // #1239: the add-picker module, called wherever this file used to keep
    // its own copy of that logic. Guarded the same way `host.fields` is —
    // if the script did not load, the rest of this loop still works, just
    // without the picker's own state surviving a re-render.
    var picker = window.HfsEditorAdd;
    if (picker) picker.attach(root);

    /* ---- the round trip -------------------------------------------------- */

    /* Posts `host`'s document (or, for a plain refresh, `overrideDoc`) plus
     * one mutation, and swaps in the re-rendered panel. `updateHost` is false
     * only for `refresh`: the text it posts came from `host` already, so
     * feeding it back would be a pointless (and, for a host with its own
     * undo history, disruptive) echo. `isStale`, given, is checked right
     * before the swap — a response that fails it is dropped silently, state
     * capture and all, as though this request had never been sent. */
    function request(op, fields, overrideDoc, updateHost, isStale) {
      var form = new URLSearchParams();
      form.set("doc", overrideDoc == null ? host.getDoc() : overrideDoc);
      form.set("op", op || "");
      form.set("pane", "form");
      Object.keys(fields || {}).forEach(function (key) {
        form.set(key, fields[key]);
      });
      // #840: a host-level extra, constant across every request this loop
      // sends for this pairing (e.g. `hidden`/`legend`) - distinct from
      // `fields` above, this call's own per-operation fields (`path`,
      // `value`, ...).
      Object.keys(host.fields || {}).forEach(function (key) {
        form.set(key, host.fields[key]);
      });
      return fetch(renderUrl, { method: "POST", body: form })
        .then(function (response) {
          return response.text();
        })
        .then(function (html) {
          if (isStale && isStale()) return;
          var state = captureUiState();
          swap(html);
          restoreUiState(state);
          if (updateHost) {
            var pretty = root.querySelector("#editor-pretty");
            if (pretty) host.setDoc(pretty.value);
          }
        });
    }

    function send(op, fields) {
      return request(op, fields, null, true, null);
    }

    /* `refresh` alone is guarded against out-of-order responses: it is the
     * one operation a caller can reasonably fire again before the last call
     * settles (JSON -> form sync re-arms on every keystroke) — a mutation
     * (`send`, above) is one discrete user action at a time, the
     * same as the standalone editor and the Resources modal this loop was
     * extracted from, neither of which has ever needed this. A response
     * that arrives after a later `refresh` already landed is simply
     * discarded — swapping it in now would show the panel a step behind
     * the document the caller has already moved on to. */
    var refreshSeq = 0;

    /* Replaces the hidden state, the guided-form card, and this render's
     * `<datalist>`s with `html`'s versions — the same three pieces
     * `partials/editor-form-fragment.html` renders together, whether the
     * response is a fresh set of rows or the invalid-JSON notice. Anything
     * `html` does not carry (there is always exactly one of each) is left as
     * it was. */
    function swap(html) {
      var fresh = new DOMParser().parseFromString(html, "text/html");

      var freshState = fresh.querySelector("#editor-form");
      var oldState = root.querySelector("#editor-form");
      if (freshState && oldState) oldState.replaceWith(freshState);
      else if (freshState) root.appendChild(freshState);

      var freshCard = fresh.querySelector("section.editor-form");
      var oldCard = root.querySelector("section.editor-form");
      if (freshCard && oldCard) oldCard.replaceWith(freshCard);
      else if (freshCard) root.appendChild(freshCard);

      root.querySelectorAll(":scope > datalist").forEach(function (list) {
        list.remove();
      });
      fresh.querySelectorAll("body > datalist").forEach(function (list) {
        root.appendChild(list);
      });
    }

    /* ---- keeping the user's place across the swap (#547) ------------------ */

    /* The whole card re-renders on every mutation; without this, each round
     * trip would destroy the focused field, the caret, any open add-picker
     * with its filter text, and the tree's scroll position. Captured at
     * response time — where the user is *now*, not where they were at
     * request time. No raw-mode bookkeeping here: unlike the standalone
     * editor and the Resources modal, a `pane=form` host keeps its own JSON
     * view outside `root` entirely — there is no raw textarea inside it to
     * track. */
    function captureUiState() {
      var state = { focus: null, pickers: [], scroll: 0 };
      var tree = root.querySelector(".editor-tree");
      if (tree) state.scroll = tree.scrollTop;
      var active = document.activeElement;
      if (active && root.contains(active) && active.dataset && active.dataset.set) {
        state.focus = {
          path: active.dataset.set,
          start: active.selectionStart,
          end: active.selectionEnd,
        };
      }
      if (picker) state.pickers = picker.capturePickers(root);
      return state;
    }

    function inputByPath(path) {
      var inputs = root.querySelectorAll("[data-set]");
      for (var i = 0; i < inputs.length; i++) {
        if (inputs[i].dataset.set === path) return inputs[i];
      }
      return null;
    }

    function restoreUiState(state) {
      // The server names the node the mutation created; the picker that
      // created it clears its filter and shows the added signal, #1239.
      var formEl = root.querySelector("#editor-form");
      var createdPath = formEl && formEl.dataset ? formEl.dataset.focus : null;
      if (picker) picker.restorePickers(root, state.pickers, createdPath);

      // The caret goes to the node the mutation created. Otherwise it
      // returns to the field that was focused before the swap.
      var target = createdPath ? inputByPath(createdPath) : null;
      if (target) {
        target.focus();
        if (target.select) target.select();
      } else if (state.focus) {
        target = inputByPath(state.focus.path);
        if (target) {
          target.focus();
          if (target.setSelectionRange && state.focus.start !== null) {
            try {
              target.setSelectionRange(state.focus.start, state.focus.end);
            } catch (ignored) {}
          }
        }
      }

      var tree = root.querySelector(".editor-tree");
      if (tree) tree.scrollTop = state.scroll;
    }

    /* ---- structural mutations: each is one round trip ------------------ */

    root.addEventListener("click", function (event) {
      var add = event.target.closest("[data-add]");
      if (add) {
        send("add", { path: add.dataset.add, name: add.dataset.name, slice: add.dataset.slice || "" });
        return;
      }

      var remove = event.target.closest("[data-remove]");
      if (remove) {
        send("remove", { path: remove.dataset.remove });
        return;
      }

      var extension = event.target.closest("[data-extension]");
      if (extension) {
        var url = picker ? picker.extensionUrl(extension) : "";
        send("extension", { path: extension.dataset.extension, url: url });
      }
    });

    // A value[x]: the user picks the type, the server creates the branch.
    root.addEventListener("change", function (event) {
      var choose = event.target.closest("[data-choose]");
      if (choose && choose.value) {
        send("choose", {
          path: choose.dataset.choose,
          name: choose.dataset.declarer,
          arm: choose.value,
        });
      }
    });

    // Primitive edits do not round-trip per keystroke — only on blur, when
    // the value is settled. The server re-validates, so the error appears
    // where the mistake is. An unchanged value needs no round trip —
    // tabbing through fields must not re-render the panel (#547).
    root.addEventListener(
      "blur",
      function (event) {
        var input = event.target.closest("[data-set]");
        if (!input) return;
        if (input.value === input.defaultValue) return;
        send("set", { path: input.dataset.set, value: input.value });
      },
      true,
    );

    /* Live $expand picker (#365): bound inputs carry data-vs-url; typing
     * debounces a request to the UI's terminology proxy and fills a per-row
     * datalist. 204 (no server configured) leaves the plain input alone.
     * The add-picker's own typeahead is `window.HfsEditorAdd.attach(root)`,
     * wired once above (#1239). */
    var expandTimer = null;
    var expandSeq = 0;
    var liveListSeq = 0;
    root.addEventListener("input", function (event) {
      var input = event.target.closest("[data-vs-url]");
      if (!input) return;
      clearTimeout(expandTimer);
      expandTimer = setTimeout(function () {
        var seq = ++expandSeq;
        fetch(
          "/ui/editor/expand?url=" +
            encodeURIComponent(input.dataset.vsUrl) +
            "&filter=" +
            encodeURIComponent(input.value),
          { credentials: "same-origin" },
        )
          .then(function (r) {
            return r.status === 200 ? r.json() : null;
          })
          .then(function (data) {
            if (!data || seq !== expandSeq) return;
            var listId = input.getAttribute("list");
            if (!listId) {
              listId = "vs-live-" + ++liveListSeq;
              input.setAttribute("list", listId);
            }
            var list = document.getElementById(listId);
            if (!list) {
              list = document.createElement("datalist");
              list.id = listId;
              input.parentElement.appendChild(list);
            }
            list.textContent = "";
            data.codes.forEach(function (item) {
              var opt = document.createElement("option");
              opt.value = item.code;
              if (item.display) opt.label = item.display;
              list.appendChild(opt);
            });
          })
          .catch(function () {});
      }, 300);
    });

    return {
      refresh: function (text) {
        var seq = ++refreshSeq;
        return request("", {}, text, false, function () {
          return seq !== refreshSeq;
        });
      },
    };
  }

  window.HfsEditorForm = { attach: attach };
})();
