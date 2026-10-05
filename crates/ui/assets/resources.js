/*
 * Resources workspace (#282): the Edit Resource modal, and "Create new".
 *
 * The search, the type rail, and the results table are the same components the
 * Search page uses (saved-queries.js), so this script owns only the modal: it
 * opens on a result click, loads the resource into the schema-driven editor
 * (the same /ui/editor/render the Editor page posts to), and wires Save, Delete,
 * and the version-history diff over the ordinary FHIR API. Nothing here talks to
 * storage directly.
 */
(function () {
  "use strict";

  /* The effective tenant, stamped by the server (#344); FHIR calls carry it. */
  var TENANT = (document.querySelector('meta[name="hfs-tenant"]') || {}).content || "";
  function fhirHeaders(extra) {
    var h = { Accept: "application/fhir+json" };
    if (TENANT) h["X-Tenant-ID"] = TENANT;
    if (extra) for (var k in extra) h[k] = extra[k];
    return h;
  }

  var root = document.getElementById("resources");
  var modal = document.getElementById("resource-modal");
  if (!root || !modal || !window.fetch) return;

  var messages = modal.dataset;
  var subject = document.getElementById("resource-modal-subject");
  var status = document.getElementById("resource-modal-status");
  var editorBody = document.getElementById("resource-editor-body");

  var current = { type: "", id: "" };

  /* The header line. `setSubject` is the plain form (`Type/id`, `Type · new`);
   * while a new resource is being created `refreshSubject` swaps it for the
   * "will be saved as" notice when the document carries a valid id (#1751). */
  function setSubject(text) {
    subject.classList.remove("subject--target");
    subject.textContent = text;
  }

  function refreshSubject() {
    if (current.id || !current.type) return;
    var doc = currentDoc();
    var text = window.HfsSaveTarget.notice(current.type, doc, messages.msgSaveTarget || "{target}");
    if (!text) { setSubject(current.type + " \u00b7 new"); return; }
    var at = text.indexOf(current.type + "/" + doc.id);
    var code = document.createElement("code");
    code.textContent = current.type + "/" + doc.id;
    subject.textContent = "";
    subject.appendChild(document.createTextNode(text.slice(0, at)));
    subject.appendChild(code);
    subject.appendChild(document.createTextNode(text.slice(at + code.textContent.length)));
    subject.classList.add("subject--target");
  }

  editorBody.addEventListener("input", function (event) {
    if (event.target.id === "editor-source") refreshSubject();
  });

  /* Pending edits (#1240): a guided-form `[data-set]` control only
   * round-trips through `editorSend("set", …)` on blur (below), so
   * #editor-doc alone lags a keystroke behind what is actually on screen.
   * One "path=value" line per control whose value has moved from what it
   * loaded with — a `select`'s loaded state is its `defaultSelected` option,
   * every other control's is `defaultValue`. Same shape as editor.js's own
   * `pendingEdits`, over the modal's own editor body. */
  function pendingEdits(container) {
    var lines = "";
    var fields = container.querySelectorAll("[data-set]");
    for (var i = 0; i < fields.length; i++) {
      var el = fields[i];
      var path = el.dataset.set;
      if (!path) continue;
      if (el.tagName === "SELECT") {
        var selected = el.options[el.selectedIndex];
        if (selected && !selected.defaultSelected) lines += path + "=" + el.value + "\n";
      } else if ("defaultValue" in el) {
        if (el.value !== el.defaultValue) lines += path + "=" + el.value + "\n";
      }
    }
    return lines;
  }

  /* The unsaved-changes tracker's own `read` (#1240): the document text plus
   * any pending edit. With one pending, the whole string no longer parses as
   * JSON, so it always differs from the last clean baseline — a pending edit
   * is dirty by definition — until it either commits (the next render
   * replaces #editor-doc and clears it) or is retyped back to its loaded
   * value.
   *
   * A hidden modal always reads "" — a `change`/`blur` the × click itself
   * causes can still schedule the tracker's own rAF-coalesced `check()`
   * *after* `closeModal()` runs when mousedown and click land in the same
   * frame (a fast click, a tap, Playwright's own click), so that check must
   * see the closed modal as clean rather than re-reading a pending field the
   * user can no longer act on — `closeModal()` resets the baseline to this
   * same "" for exactly that reason. */
  function readWithPending() {
    if (modal.hidden) return "";
    var pending = pendingEdits(editorBody);
    return currentDocText() + (pending ? "\n--pending--\n" + pending : "");
  }

  /* Unsaved-changes tracking (#1240): one tracker for the modal's whole
   * lifetime — `openResource`/`openNew` reset its baseline once each render
   * lands, `editorSend` re-checks it on every swap. */
  var unsaved = window.HfsUnsaved
    ? window.HfsUnsaved.track({
        root: modal,
        read: readWithPending,
        cue: modal.querySelector(".modal__actions"),
      })
    : null;

  /* ---- open / close ---------------------------------------------------- */

  function openModal() {
    modal.hidden = false;
    document.body.style.overflow = "hidden";
    showTab("edit");
    status.textContent = "";
    status.className = "modal__status";
    // The clicked result id keeps focus and hover behind the modal; let the
    // shared tooltip (resource-filter.js) hide its full-id bubble (#1770).
    document.dispatchEvent(new CustomEvent("hfs:modal-open"));
  }
  function closeModal() {
    modal.hidden = true;
    document.body.style.overflow = "";
    // A hidden modal is never dirty (#1240): reset (not markClean) so the
    // baseline itself becomes "" — readWithPending() already reads "" while
    // hidden, so a check() already queued (or the late round trip of a blur
    // this same close caused) lands on baseline "" === read "" and computes
    // clean no matter when it actually runs.
    if (unsaved) unsaved.reset();
  }

  // #1240: ask before discarding the modal's edits. The answer comes back
  // asynchronously from the shared in-page confirmation (#1667); a clean
  // modal resolves at once.
  function closeAskingFirst() {
    var asked = window.HfsUnsaved ? window.HfsUnsaved.confirmDiscard(modal) : Promise.resolve(true);
    asked.then(function (discard) {
      if (discard && !modal.hidden) closeModal();
    });
  }

  modal.addEventListener("click", function (event) {
    if (event.target.closest("[data-modal-close]")) closeAskingFirst();
    var tab = event.target.closest("[data-modal-tab]");
    if (tab) showTab(tab.dataset.modalTab);
  });
  document.addEventListener("keydown", function (event) {
    if (event.key === "Escape" && !modal.hidden) {
      // preventDefault: a confirmation opened inside this keydown would
      // otherwise be dismissed by the very same Escape (the browser's own
      // <dialog> close handling runs after the listeners), answering "cancel"
      // before the user sees it (#1667).
      event.preventDefault();
      closeAskingFirst();
    }
  });

  function showTab(name) {
    modal.querySelectorAll("[data-modal-pane]").forEach(function (pane) {
      pane.hidden = pane.dataset.modalPane !== name;
    });
    modal.querySelectorAll("[data-modal-tab]").forEach(function (tab) {
      var on = tab.dataset.modalTab === name;
      tab.classList.toggle("modal__tab--on", on);
      tab.setAttribute("aria-selected", on ? "true" : "false");
    });
    if (name === "history") loadHistory();
  }

  /* ---- load a resource into the embedded editor ------------------------ */

  function renderEditor(resource) {
    return editorSend("", { doc: JSON.stringify(resource) });
  }

  /* The editor inside the modal is the same fragment the Editor page uses, but
   * editor.js is bound to that page's ids — so the interactions live here,
   * scoped to the modal body. Each structural edit posts the whole document
   * plus the op and swaps the body, exactly as the Editor page does. */
  function editorSend(op, fields, operation) {
    var picker = window.HfsEditorAdd;
    if (op && picker.projectionBusy(editorBody)) return Promise.resolve();
    var version = picker.documentVersion(editorBody);
    function work() {
      var form = new URLSearchParams();
      var docField = editorBody.querySelector("#editor-doc");
      form.set("doc", (fields && fields.doc) || (docField ? docField.value : "{}"));
      form.set("op", op || "");
      Object.keys(fields || {}).forEach(function (k) { if (k !== "doc") form.set(k, fields[k]); });
      return fetch("/ui/editor/render", { method: "POST", body: form })
        .then(function (r) { if (!r.ok) throw new Error(String(r.status)); return r.text(); })
        .then(function (html) {
          if (!op && version !== picker.documentVersion(editorBody)) return;
          var state = captureEditorState();
          var fresh = new DOMParser().parseFromString(html, "text/html");
          if (!fresh.querySelector("#editor-form")) throw new Error("Invalid editor render response");
          editorBody.innerHTML = html;
          picker.projectionSwapped(editorBody, op);
          restoreEditorState(state, operation);
          refreshSubject();
          if (unsaved && !modal.hidden) unsaved.check();
        });
    }
    var request;
    if (op) request = picker.queueMutation(editorBody, work, function () { picker.failedMutation(editorBody, op, fields); }, op);
    else {
      var finish = picker.beginRequest(editorBody);
      request = work().finally(finish);
    }
    return request.catch(function (error) { console.debug("Resource editor render failed", error); });
  }

  /* Keeps the user's place across the modal editor's full re-render (#547):
   * the focused field and caret, any open add-picker with its filter text,
   * and the tree scroll. The server marks the node a mutation created via
   * data-focus on #editor-form; the caret goes there first. */
  function captureEditorState() {
    var state = { focus: null, pickers: [], scroll: 0, rawOpen: false };
    var rawPane = editorBody.querySelector("#editor-json-raw");
    state.rawOpen = !!(rawPane && !rawPane.hidden);
    var tree = editorBody.querySelector(".editor-tree");
    if (tree) state.scroll = tree.scrollTop;
    var active = document.activeElement;
    if (active && editorBody.contains(active) && active.dataset && active.dataset.set) {
      state.focus = { path: active.dataset.set, start: active.selectionStart, end: active.selectionEnd };
    }
    state.pickers = window.HfsEditorAdd.capturePickers(editorBody);
    return state;
  }

  function editorNodeBy(attr, path) {
    var nodes = editorBody.querySelectorAll("[" + attr + "]");
    for (var i = 0; i < nodes.length; i++) {
      if (nodes[i].getAttribute(attr) === path) return nodes[i];
    }
    return null;
  }

  function restoreEditorState(state, operation) {
    // Raw mode survives the swap: the fresh textarea already carries the
    // updated document, so a guided edit refreshes the JSON in place instead
    // of kicking the user back to the fold view.
    if (state.rawOpen) {
      var rawPane = editorBody.querySelector("#editor-json-raw");
      var viewEl = editorBody.querySelector("#json-view");
      var toggle = editorBody.querySelector("#editor-json-edit");
      if (rawPane && viewEl) {
        rawPane.hidden = false;
        viewEl.hidden = true;
        if (toggle) toggle.classList.add("editor-json__act--on");
      }
    }
    var formEl = editorBody.querySelector("#editor-form");
    var createdPath = formEl && formEl.dataset ? formEl.dataset.focus : null;
    window.HfsEditorAdd.restorePickers(editorBody, state.pickers, createdPath, operation);
    if (window.HfsEditorAdd.revealCreated(editorBody, createdPath, operation)) return;

    var target = state.focus ? editorNodeBy("data-set", state.focus.path) : null;
    if (target) {
      target.focus({ preventScroll: true });
      if (target.setSelectionRange && state.focus.start !== null) {
        try { target.setSelectionRange(state.focus.start, state.focus.end); } catch (ignored) {}
      }
    }

    var tree = editorBody.querySelector(".editor-tree");
    if (tree) tree.scrollTop = state.scroll;
    window.HfsEditorAdd.restoreUndoFocus(editorBody, operation);
  }

  /* Delegated editor interactions within the modal body. */
  editorBody.addEventListener("click", function (event) {
    if (event.target.id === "editor-json-edit") {
      var raw = editorBody.querySelector("#editor-json-raw");
      var viewEl = editorBody.querySelector("#json-view");
      if (!raw || !viewEl) return;
      if (raw.hidden) { raw.hidden = false; viewEl.hidden = true; event.target.classList.add("editor-json__act--on"); }
      else {
        // Same as the standalone page: close the pane before the round trip
        // so the raw-mode persistence reads this re-render as leaving raw.
        var src = editorBody.querySelector("#editor-source");
        var fld = editorBody.querySelector("#editor-doc");
        if (src && fld) fld.value = src.value;
        raw.hidden = true;
        viewEl.hidden = false;
        event.target.classList.remove("editor-json__act--on");
        editorSend("");
      }
      return;
    }
    var add = event.target.closest("[data-add]");
    if (add) { editorSend("add", { path: add.dataset.add, name: add.dataset.name, slice: add.dataset.slice || "" }, window.HfsEditorAdd.operationFrom(add)); return; }
    var rm = event.target.closest("[data-remove]");
    if (rm) {
      var doc = editorBody.querySelector("#editor-doc");
      var raw = editorBody.querySelector("#editor-json-raw");
      var source = editorBody.querySelector("#editor-source");
      var current = raw && !raw.hidden && source ? source.value : doc ? doc.value : "{}";
      var removal = rm.hasAttribute("data-add-undo") ? window.HfsEditorAdd.undoOperation(editorBody, rm, current) : null;
      if (rm.hasAttribute("data-add-undo") && !removal) return;
      editorSend("remove", { path: rm.dataset.remove }, removal);
      return;
    }
    var ext = event.target.closest("[data-extension]");
    if (ext) {
      var url = window.HfsEditorAdd.extensionUrl(ext);
      editorSend("extension", { path: ext.dataset.extension, url: url }, window.HfsEditorAdd.operationFrom(ext));
    }
  });


  /* Live $expand picker (#365): bound inputs carry data-vs-url; typing
   * debounces a request to the UI's terminology proxy and fills a per-row
   * datalist. 204 (no server configured) leaves the plain input alone. */
  var expandTimer = null;
  var expandSeq = 0;
  var liveListSeq = 0;
  editorBody.addEventListener("input", function (event) {
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
        .then(function (r) { return r.status === 200 ? r.json() : null; })
        .then(function (data) {
          if (!data || seq !== expandSeq) return;
          var listId = input.getAttribute("list");
          if (!listId) {
            listId = "vs-live-" + (++liveListSeq);
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

  editorBody.addEventListener("change", function (event) {
    var choose = event.target.closest("[data-choose]");
    if (choose && choose.value) {
      editorSend("choose", { path: choose.dataset.choose, name: choose.dataset.declarer, arm: choose.value }, window.HfsEditorAdd.operationFrom(choose));
    }
  });

  editorBody.addEventListener("blur", function (event) {
    var input = event.target.closest("[data-set]");
    if (!input) return;
    // An unchanged value needs no round trip (#547).
    if (input.value === input.defaultValue) return;
    editorSend("set", { path: input.dataset.set, value: input.value });
  }, true);

  /* The add-picker's own typeahead over the "add" list (#1239). */
  window.HfsEditorAdd.attach(editorBody);

  function openResource(type, id) {
    current = { type: type, id: id };
    setSubject(type + "/" + id);
    openModal();
    editorBody.innerHTML = "";
    fetch("/" + type + "/" + id, { headers: fhirHeaders() })
      .then(function (r) { if (!r.ok) throw new Error(String(r.status)); return r.json(); })
      .then(renderEditor)
      .then(function () { if (unsaved) unsaved.reset(); })
      .catch(function () { say(messages.msgLoadError, "error"); });
  }

  function openNew(type) {
    if (
      !type ||
      root.dataset.createEligible !== "true" ||
      root.dataset.createTarget !== type
    ) return;
    current = { type: type, id: "" };
    setSubject(type + " · " + "new");
    openModal();
    editorBody.innerHTML = "";
    renderEditor({ resourceType: type }).then(function () { if (unsaved) unsaved.reset(); });
  }

  /* Clicking a result row opens it. The href remains the server-provided
   * public URL, which may include a path prefix or tenant segment. Use the
   * trusted resource identity attached by saved-queries.js instead of parsing
   * that deployment-specific URL. The results live in the content column, not
   * under `root` (the type panel), so the listener is on the document.
   * row-navigation.js (#1106) turns a click anywhere in the row into a click
   * on this id link, so this capture-phase interceptor opens the modal for
   * whole-row clicks too. */
  document.addEventListener(
    "click",
    function (event) {
      var link = event.target.closest("#query-results-body a.result-id");
      if (!link) return;
      var type = link.dataset.resourceType || "";
      var id = link.dataset.resourceId || "";
      if (!/^[A-Za-z]+$/.test(type) || !/^[A-Za-z0-9.-]{1,64}$/.test(id)) return;
      event.preventDefault();
      openResource(type, id);
    },
    true
  );

  var createBtn = document.getElementById("resource-create");
  if (createBtn) {
    createBtn.addEventListener("click", function () {
      // `panel.dataset.selectedType` (the rail's `<aside>`) is the single
      // source of truth for the selected type (#605) — the button carries no
      // type of its own any more.
      openNew(root.dataset.selectedType);
    });
  }

  /* ---- save / delete --------------------------------------------------- */

  // When the raw editor is open its textarea is the source of truth: the
  // hidden #editor-doc only catches up when you toggle "Edit raw" back off,
  // so a Save typed directly in raw mode must read the textarea itself.
  function currentDocText() {
    var raw = editorBody.querySelector("#editor-json-raw");
    var source = editorBody.querySelector("#editor-source");
    if (raw && !raw.hidden && source) return source.value;
    var field = editorBody.querySelector("#editor-doc");
    return field ? field.value : "{}";
  }

  function currentDoc() {
    try { return JSON.parse(currentDocText()); } catch (e) { return null; }
  }

  document.getElementById("resource-save").addEventListener("click", function () {
    var doc = currentDoc();
    if (!doc) { say(messages.msgSaveInvalid, "error"); return; }
    // Validate the exact document being saved by re-rendering it (this also
    // commits a raw edit and leaves raw mode), then block the save if the
    // editor reports any issue — an invalid resource must not be persisted.
    // The whole chain runs under the shared busy state (#1750): repeat clicks
    // are dropped until it settles, whatever the outcome.
    var saveButton = document.getElementById("resource-save");
    var deleteButton = document.getElementById("resource-delete");
    window.hfsBusy.during([saveButton], function () {
      return editorSend("", { doc: JSON.stringify(doc) }).then(function () {
        var form = editorBody.querySelector("#editor-form");
        var errors = form ? parseInt(form.dataset.errorCount || "0", 10) : 0;
        if (errors > 0) { say(messages.msgSaveBlocked, "error"); return; }
        var target = current.id
          ? { method: "PUT", url: "/" + current.type + "/" + current.id }
          : window.HfsSaveTarget.forCreate(current.type, doc);
        return fetch(target.url, {
          method: target.method,
          headers: fhirHeaders({ "Content-Type": "application/fhir+json" }),
          body: JSON.stringify(doc),
        })
          .then(function (r) {
            return r.json().then(function (body) { return { ok: r.ok, body: body }; });
          })
          .then(function (res) {
            if (!res.ok) { say(outcomeText(res.body), "error"); return; }
            current.id = res.body.id || current.id;
            setSubject(current.type + "/" + current.id);
            say("");
            announce(messages.msgSaved);
            // The results table behind the modal is now stale — let it catch up.
            document.dispatchEvent(new CustomEvent("hfs:data-changed", { detail: { type: current.type } }));
            return renderEditor(res.body).then(function () { if (unsaved) unsaved.reset(); });
          })
          .catch(function () { say(messages.msgLoadError, "error"); });
      });
    }, { alsoDisable: [deleteButton] });
  });

  document.getElementById("resource-delete").addEventListener("click", function () {
    if (!current.id) {
      closeAskingFirst();
      return;
    }
    // Pin the target now: the shared in-page confirmation (#1667) answers
    // later, and `current` must not be read again after the user has said yes.
    var type = current.type;
    var id = current.id;
    window.HfsConfirm.ask(messages.msgConfirmDelete, { danger: true }).then(function (confirmed) {
      if (!confirmed) return;
      var saveButton = document.getElementById("resource-save");
      var deleteButton = document.getElementById("resource-delete");
      window.hfsBusy.during([deleteButton], function () {
        return fetch("/" + type + "/" + id, { method: "DELETE", headers: fhirHeaders() })
          .then(function (r) {
            if (r.ok || r.status === 204) {
              // The resource no longer exists — nothing to ask about.
              if (unsaved) unsaved.markClean();
              closeModal();
              // No full reload: the table and counts refresh in place, keeping
              // the rail selection and scroll where the user left them.
              document.dispatchEvent(new CustomEvent("hfs:data-changed", { detail: { type: type } }));
            } else say(String(r.status), "error");
          })
          .catch(function () { say(messages.msgLoadError, "error"); });
      }, { alsoDisable: [saveButton] });
    });
  });

  /* ---- history tab: version rail + diff (#236) ------------------------- */

  var historyEl = document.getElementById("resource-history");
  var versionsHost = document.getElementById("resource-history-versions");
  var fromSel = document.getElementById("resource-history-from");
  var toSel = document.getElementById("resource-history-to");
  var metaToggle = document.getElementById("resource-history-metadata");
  var diffHost = document.getElementById("resource-history-diff");
  var versions = [];

  function loadHistory() {
    if (!current.id) {
      diffHost.innerHTML = "<p class=\"history__empty\">—</p>";
      return;
    }
    document.getElementById("resource-history-subject").textContent =
      current.type + "/" + current.id;
    fetch("/" + current.type + "/" + current.id + "/_history", {
      headers: fhirHeaders(),
    })
      .then(function (r) { return r.ok ? r.json() : null; })
      .then(function (bundle) { renderVersions((bundle && bundle.entry) || []); })
      .catch(function () {});
  }

  function renderVersions(entries) {
    versions = entries.map(function (entry) {
      var resource = entry.resource || {};
      var response = entry.response || {};
      var etag = /"([^"]+)"/.exec(response.etag || "");
      return {
        versionId: (resource.meta && resource.meta.versionId) || (etag && etag[1]) || "",
        resource: resource,
      };
    });
    versionsHost.textContent = "";
    fromSel.textContent = "";
    toSel.textContent = "";
    versions.forEach(function (v, i) {
      var row = document.createElement("button");
      row.type = "button";
      row.className = "history-version" + (i === 0 ? " history-version--current" : "");
      row.textContent = "v" + v.versionId + (i === 0 ? " · " + historyEl.dataset.msgCurrent : "");
      row.addEventListener("click", function () { toSel.value = String(i); fromSel.value = String(Math.min(i + 1, versions.length - 1)); renderDiff(); });
      versionsHost.appendChild(row);
      fromSel.appendChild(opt(i, "v" + v.versionId));
      toSel.appendChild(opt(i, "v" + v.versionId));
    });
    document.getElementById("resource-history-controls").hidden = versions.length < 1;
    if (versions.length >= 2) { fromSel.value = "1"; toSel.value = "0"; }
    renderDiff();
  }

  function opt(v, label) { var o = document.createElement("option"); o.value = String(v); o.textContent = label; return o; }

  function renderDiff() {
    var from = versions[Number(fromSel.value)];
    var to = versions[Number(toSel.value)];
    if (!from || !to) return;
    var body = new URLSearchParams();
    body.set("from", JSON.stringify(from.resource));
    body.set("to", JSON.stringify(to.resource));
    body.set("from_label", "v" + from.versionId);
    body.set("to_label", "v" + to.versionId);
    body.set("show_metadata", metaToggle.checked ? "true" : "false");
    fetch("/ui/history/diff", { method: "POST", body: body })
      .then(function (r) { return r.text(); })
      .then(function (html) { diffHost.innerHTML = html; });
  }

  fromSel.addEventListener("change", renderDiff);
  toSel.addEventListener("change", renderDiff);
  metaToggle.addEventListener("change", renderDiff);

  /* ---- helpers --------------------------------------------------------- */

  function say(text, kind) {
    status.textContent = text;
    status.className = "modal__status modal__status--" + (kind || "");
  }
  /* The saved confirmation for assistive technology only (#1649); see
   * editor.js's `announce`. */
  var announcer = document.getElementById("resource-modal-announce");
  function announce(text) {
    if (!announcer) return;
    announcer.textContent = "";
    window.setTimeout(function () {
      announcer.textContent = text || "";
    }, 50);
  }
  function outcomeText(body) {
    return (
      (body && body.issue && body.issue[0] &&
        (body.issue[0].diagnostics || (body.issue[0].details && body.issue[0].details.text))) ||
      messages.msgLoadError
    );
  }
})();
