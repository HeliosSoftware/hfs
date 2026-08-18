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

  /* ---- open / close ---------------------------------------------------- */

  function openModal() {
    modal.hidden = false;
    document.body.style.overflow = "hidden";
    showTab("edit");
    status.textContent = "";
    status.className = "modal__status";
  }
  function closeModal() {
    modal.hidden = true;
    document.body.style.overflow = "";
  }

  modal.addEventListener("click", function (event) {
    if (event.target.closest("[data-modal-close]")) closeModal();
    var tab = event.target.closest("[data-modal-tab]");
    if (tab) showTab(tab.dataset.modalTab);
  });
  document.addEventListener("keydown", function (event) {
    if (event.key === "Escape" && !modal.hidden) closeModal();
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
  function editorSend(op, fields) {
    var form = new URLSearchParams();
    var docField = editorBody.querySelector("#editor-doc");
    form.set("doc", (fields && fields.doc) || (docField ? docField.value : "{}"));
    form.set("op", op || "");
    Object.keys(fields || {}).forEach(function (k) {
      if (k !== "doc") form.set(k, fields[k]);
    });
    return fetch("/ui/editor/render", { method: "POST", body: form })
      .then(function (r) { return r.text(); })
      .then(function (html) {
        var state = captureEditorState();
        editorBody.innerHTML = html;
        restoreEditorState(state);
      });
  }

  /* Keeps the user's place across the modal editor's full re-render (#547):
   * the focused field and caret, any open add-picker with its filter text,
   * and the tree scroll. The server marks the node a mutation created via
   * data-focus on #editor-form; the caret goes there first. */
  function captureEditorState() {
    var state = { focus: null, pickers: [], scroll: 0 };
    var tree = editorBody.querySelector(".editor-tree");
    if (tree) state.scroll = tree.scrollTop;
    var active = document.activeElement;
    if (active && editorBody.contains(active) && active.dataset && active.dataset.set) {
      state.focus = { path: active.dataset.set, start: active.selectionStart, end: active.selectionEnd };
    }
    editorBody.querySelectorAll("details.editor-add[open]").forEach(function (box) {
      var row = box.closest("[data-path]");
      var filter = box.querySelector(".editor-add__filter");
      state.pickers.push({
        path: row ? row.dataset.path : "",
        filter: filter ? filter.value : "",
        focusFilter: filter === document.activeElement,
      });
    });
    return state;
  }

  function editorNodeBy(attr, path) {
    var nodes = editorBody.querySelectorAll("[" + attr + "]");
    for (var i = 0; i < nodes.length; i++) {
      if (nodes[i].getAttribute(attr) === path) return nodes[i];
    }
    return null;
  }

  function restoreEditorState(state) {
    state.pickers.forEach(function (saved) {
      var row = saved.path ? editorNodeBy("data-path", saved.path) : editorBody;
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

    var formEl = editorBody.querySelector("#editor-form");
    var createdPath = formEl && formEl.dataset ? formEl.dataset.focus : null;
    var target = createdPath ? editorNodeBy("data-set", createdPath) : null;
    if (target) {
      target.focus();
      if (target.select) target.select();
    } else if (state.focus) {
      target = editorNodeBy("data-set", state.focus.path);
      if (target) {
        target.focus();
        if (target.setSelectionRange && state.focus.start !== null) {
          try { target.setSelectionRange(state.focus.start, state.focus.end); } catch (ignored) {}
        }
      }
    }

    var tree = editorBody.querySelector(".editor-tree");
    if (tree) tree.scrollTop = state.scroll;
  }

  /* Delegated editor interactions within the modal body. */
  editorBody.addEventListener("click", function (event) {
    var arrow = event.target.closest("[data-fold]");
    if (arrow) { toggleFold(arrow.dataset.fold); return; }
    var foldAllBtn = event.target.closest("[data-json-fold]");
    if (foldAllBtn) { setAllFolds(foldAllBtn.dataset.jsonFold === "all"); return; }
    if (event.target.id === "editor-json-edit") {
      var raw = editorBody.querySelector("#editor-json-raw");
      var viewEl = editorBody.querySelector("#json-view");
      if (!raw || !viewEl) return;
      if (raw.hidden) { raw.hidden = false; viewEl.hidden = true; event.target.classList.add("editor-json__act--on"); }
      else {
        var src = editorBody.querySelector("#editor-source");
        var fld = editorBody.querySelector("#editor-doc");
        if (src && fld) fld.value = src.value;
        editorSend("");
      }
      return;
    }
    var add = event.target.closest("[data-add]");
    if (add) { editorSend("add", { path: add.dataset.add, name: add.dataset.name, slice: add.dataset.slice || "" }); return; }
    var rm = event.target.closest("[data-remove]");
    if (rm) { editorSend("remove", { path: rm.dataset.remove }); return; }
    var ext = event.target.closest("[data-extension]");
    if (ext) {
      var panel = ext.closest(".editor-add__ext");
      var url = ext.dataset.url || (panel ? panel.querySelector(".editor-add__ext-url").value.trim() : "");
      editorSend("extension", { path: ext.dataset.extension, url: url });
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
      editorSend("choose", { path: choose.dataset.choose, name: choose.dataset.declarer, arm: choose.value });
    }
  });

  editorBody.addEventListener("blur", function (event) {
    var input = event.target.closest("[data-set]");
    if (!input) return;
    // An unchanged value needs no round trip (#547).
    if (input.value === input.defaultValue) return;
    editorSend("set", { path: input.dataset.set, value: input.value });
  }, true);

  editorBody.addEventListener("input", function (event) {
    var filter = event.target.closest(".editor-add__filter");
    if (!filter) return;
    var needle = filter.value.trim().toLowerCase();
    filter.closest(".editor-add__panel").querySelectorAll("[data-add-name]").forEach(function (item) {
      item.hidden = needle && item.dataset.addName.toLowerCase().indexOf(needle) < 0;
    });
  });

  /* JSON fold helpers, scoped to the modal body. */
  function toggleFold(foldId) {
    var opener = editorBody.querySelector('.json-line[data-fold-id="' + foldId + '"]');
    if (!opener) return;
    opener.classList.toggle("json-line--collapsed", !opener.classList.contains("json-line--collapsed"));
    reflowFolds();
  }
  function setAllFolds(collapse) {
    editorBody.querySelectorAll(".json-line--foldable").forEach(function (o) {
      if (o.dataset.parents) o.classList.toggle("json-line--collapsed", collapse);
    });
    reflowFolds();
  }
  function reflowFolds() {
    editorBody.querySelectorAll(".json-line").forEach(function (line) {
      var parents = (line.dataset.parents || "").split(" ");
      line.hidden = parents.some(function (p) {
        if (!p) return false;
        var o = editorBody.querySelector('.json-line[data-fold-id="' + p + '"]');
        return o && o.classList.contains("json-line--collapsed");
      });
    });
  }

  function openResource(type, id) {
    current = { type: type, id: id };
    subject.textContent = type + "/" + id;
    openModal();
    editorBody.innerHTML = "";
    fetch("/" + type + "/" + id, { headers: fhirHeaders() })
      .then(function (r) { if (!r.ok) throw new Error(String(r.status)); return r.json(); })
      .then(renderEditor)
      .catch(function () { say(messages.msgLoadError, "error"); });
  }

  function openNew(type) {
    current = { type: type, id: "" };
    subject.textContent = type + " · " + "new";
    openModal();
    editorBody.innerHTML = "";
    renderEditor({ resourceType: type });
  }

  /* Clicking a result row opens it. The results table (from saved-queries.js)
   * renders id links as `/{type}/{id}`; intercept them into the modal. The
   * results live in the content column, not under `root` (the type panel), so
   * the listener is on the document. */
  document.addEventListener(
    "click",
    function (event) {
      var link = event.target.closest("#query-results-body a.url");
      if (!link) return;
      var m = /^\/([A-Za-z]+)\/([^/?]+)/.exec(link.getAttribute("href") || "");
      if (!m) return;
      event.preventDefault();
      openResource(m[1], m[2]);
    },
    true
  );

  var createBtn = document.getElementById("resource-create");
  if (createBtn) {
    createBtn.addEventListener("click", function () {
      openNew(root.dataset.selectedType || createBtn.dataset.type || "Patient");
    });
  }

  /* ---- save / delete --------------------------------------------------- */

  function currentDoc() {
    // When the raw editor is open its textarea is the source of truth: the
    // hidden #editor-doc only catches up when you toggle "Edit raw" back off,
    // so a Save typed directly in raw mode must read the textarea itself.
    var raw = editorBody.querySelector("#editor-json-raw");
    var source = editorBody.querySelector("#editor-source");
    if (raw && !raw.hidden && source) {
      try { return JSON.parse(source.value); } catch (e) { return null; }
    }
    var field = editorBody.querySelector("#editor-doc");
    if (!field) return null;
    try { return JSON.parse(field.value); } catch (e) { return null; }
  }

  document.getElementById("resource-save").addEventListener("click", function () {
    var doc = currentDoc();
    if (!doc) { say(messages.msgSaveInvalid, "error"); return; }
    // Validate the exact document being saved by re-rendering it (this also
    // commits a raw edit and leaves raw mode), then block the save if the
    // editor reports any issue — an invalid resource must not be persisted.
    editorSend("", { doc: JSON.stringify(doc) }).then(function () {
      var form = editorBody.querySelector("#editor-form");
      var errors = form ? parseInt(form.dataset.errorCount || "0", 10) : 0;
      if (errors > 0) { say(messages.msgSaveBlocked, "error"); return; }
      var creating = !current.id;
      var url = creating ? "/" + current.type : "/" + current.type + "/" + current.id;
      fetch(url, {
        method: creating ? "POST" : "PUT",
        headers: fhirHeaders({ "Content-Type": "application/fhir+json" }),
        body: JSON.stringify(doc),
      })
        .then(function (r) {
          return r.json().then(function (body) { return { ok: r.ok, body: body }; });
        })
        .then(function (res) {
          if (!res.ok) { say(outcomeText(res.body), "error"); return; }
          current.id = res.body.id || current.id;
          subject.textContent = current.type + "/" + current.id;
          say(messages.msgSaved, "ok");
          renderEditor(res.body);
        })
        .catch(function () { say(messages.msgLoadError, "error"); });
    });
  });

  document.getElementById("resource-delete").addEventListener("click", function () {
    if (!current.id) { closeModal(); return; }
    if (!window.confirm(messages.msgConfirmDelete)) return;
    fetch("/" + current.type + "/" + current.id, { method: "DELETE", headers: fhirHeaders() })
      .then(function (r) {
        if (r.ok || r.status === 204) { closeModal(); location.reload(); }
        else say(String(r.status), "error");
      })
      .catch(function () { say(messages.msgLoadError, "error"); });
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
  function outcomeText(body) {
    return (
      (body && body.issue && body.issue[0] &&
        (body.issue[0].diagnostics || (body.issue[0].details && body.issue[0].details.text))) ||
      messages.msgLoadError
    );
  }
})();
