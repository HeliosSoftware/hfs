/*
 * Resource editor (#264).
 *
 * This script is deliberately thin, and that is the whole architectural point.
 * It does not model the resource, it does not know what a choice type is, it
 * has never heard of cardinality, and it cannot tell an extension from a family
 * name. All of that lives in Rust, behind /ui/editor/render, where it is tested.
 *
 * What the script does:
 *   1. fetches the resource from the ordinary FHIR API,
 *   2. hands the document to the server with whatever the user just did,
 *   3. swaps in the HTML the server hands back,
 *   4. saves it again through the ordinary FHIR API.
 *
 * The document lives in a hidden field inside the fragment the server renders.
 * It is the single copy. Nothing here derives from it, so nothing here can lose
 * a key it did not understand -- which is the failure mode of every schema-driven
 * editor we surveyed for #264.
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

  var root = document.getElementById("editor");
  if (!root || !window.fetch) return;

  var body = document.getElementById("editor-body");
  var status = document.getElementById("editor-status");
  var subject = document.getElementById("editor-subject");
  var messages = root.dataset;

  /* Unsaved-changes tracking (#1240): opt in lazily, once the document is in
   * scope, so `read` never runs before the fragment it reads exists. */
  var unsaved = null;
  function trackUnsaved() {
    if (unsaved || !window.HfsUnsaved) return;
    unsaved = window.HfsUnsaved.track({
      root: root,
      checkOnExit: true,
      read: readWithPending,
      cue: root.querySelector(".editor__actions"),
    });
  }

  var resourceType = messages.type;
  var resourceId = messages.id;
  var confirmed = null;
  var saving = false;
  var ready = false;
  var canonicalPending = null;
  var editRevision = 0;
  var rawReplacement = null;
  var saveButton = document.getElementById("editor-save");
  var deleteButton = document.getElementById("editor-delete");
  function updateActions() {
    saveButton.disabled = saving || !ready;
    deleteButton.hidden = !confirmed;
    deleteButton.disabled = saving;
    body.inert = saving || !!canonicalPending;
  }
  function confirmIdentity(resource) {
    if (!resource || resource.resourceType !== resourceType ||
        !/^[A-Za-z0-9.-]{1,64}$/.test(resource.id || "")) return;
    confirmed = { type: resourceType, id: resource.id, url: resource.url, code: resource.code };
    resourceId = resource.id;
    subject.textContent = resourceType + "/" + resourceId +
      (resource.meta && resource.meta.lastUpdated
        ? " · " + new Date(resource.meta.lastUpdated).toLocaleString() : "");
    updateActions();
  }
  root.addEventListener("input", function (event) {
    if (!event.target.matches("[data-set], #editor-source")) return;
    editRevision++;
    window.HfsEditorAdd.invalidateRefresh(body);
    rawReplacement = null;
    if (event.target.id === "editor-source") {
      try {
        var replacement = JSON.stringify(JSON.parse(event.target.value));
        var previous = event.target.defaultValue;
        try { previous = JSON.stringify(JSON.parse(previous)); } catch (invalidPrevious) {}
        if (replacement !== previous) {
          rawReplacement = { doc: event.target.value, version: window.HfsEditorAdd.documentVersion(body) };
        }
      } catch (invalidJson) { /* Save reports invalid raw JSON normally. */ }
    }
  });
  updateActions();

  function say(text, kind) {
    status.textContent = text || "";
    status.className = "editor__status" + (kind ? " editor__status--" + kind : "");
  }

  /* A successful save is confirmed by the "Unsaved changes" pill going away,
   * not by visible text (#1649). `announce` hands the words to assistive
   * technology through the page's visually hidden live region; emptying it
   * first makes a repeated save re-announce instead of leaving identical
   * text in place. */
  var announcer = document.getElementById("editor-announce");
  function announce(text) {
    if (!announcer) return;
    announcer.textContent = "";
    window.setTimeout(function () {
      if (root.isConnected) announcer.textContent = text || "";
    }, 50);
  }

  /* ---- the round trip -------------------------------------------------- */

  /* Posts the document plus one mutation, and swaps in the re-rendered body.
   * `op` is empty for a plain re-render (first load, or after a source edit). */
  function send(op, fields, operation) {
    var picker = window.HfsEditorAdd;
    if (op && picker.projectionBusy(body)) return Promise.resolve(false);
    var version = picker.documentVersion(body);
    function work() {
      var form = new URLSearchParams();
      // Read after earlier queued mutations have applied their server swap.
      form.set("doc", currentDocument());
      form.set("op", op || "");
      Object.keys(fields || {}).forEach(function (key) { form.set(key, fields[key]); });
      return fetch("/ui/editor/render", { method: "POST", body: form })
        .then(function (response) {
          if (!response.ok) throw new Error(String(response.status));
          return response.text();
        })
        .then(function (html) {
          if (!root.isConnected || (!op && version !== picker.documentVersion(body))) return false;
          var state = captureUiState();
          var fresh = new DOMParser().parseFromString(html, "text/html");
          if (!fresh.querySelector("#editor-form")) throw new Error("Invalid editor render response");
          body.innerHTML = html;
          picker.projectionSwapped(body, op);
          // Once an explicitly authored replacement is the current projection,
          // later formatting edits must not resurrect the superseded failure.
          var projectedDoc = fresh.querySelector("#editor-doc");
          if (!op && rawReplacement && rawReplacement.version === version && projectedDoc &&
              picker.canonicalDocument(projectedDoc.value) === picker.canonicalDocument(rawReplacement.doc)) {
            picker.supersedeCompletedMutations(body);
          }
          applyView();
          restoreUiState(state, operation);
          if (unsaved) unsaved.check();
          return true;
        });
    }
    var request;
    if (op) request = picker.queueMutation(body, work, function () { picker.failedMutation(body, op, fields); }, op);
    else {
      var finish = picker.beginRequest(body);
      request = work().finally(finish);
    }
    return request.catch(function (error) { console.debug("Editor render failed", error); return false; });
  }

  /* ---- keeping the user's place across the swap (#547) ------------------ */

  /* The whole body re-renders on every mutation; without this, each round
   * trip destroyed the focused field, the caret, any open add-picker with
   * its filter text, and the tree's scroll position. Captured at response
   * time — where the user is *now*, not where they were at request time. */
  function captureUiState() {
    var state = { focus: null, pickers: [], scroll: 0, rawOpen: false };
    var raw = body.querySelector("#editor-json-raw");
    state.rawOpen = !!(raw && !raw.hidden);
    var tree = body.querySelector(".editor-tree");
    if (tree) state.scroll = tree.scrollTop;
    var active = document.activeElement;
    if (active && body.contains(active) && active.dataset && active.dataset.set) {
      state.focus = {
        path: active.dataset.set,
        start: active.selectionStart,
        end: active.selectionEnd,
      };
    }
    state.pickers = window.HfsEditorAdd.capturePickers(body);
    return state;
  }

  function inputByPath(path) {
    var inputs = body.querySelectorAll("[data-set]");
    for (var i = 0; i < inputs.length; i++) {
      if (inputs[i].dataset.set === path) return inputs[i];
    }
    return null;
  }

  function restoreUiState(state, operation) {
    // Raw mode survives the swap: the fresh textarea already carries the
    // updated document, so a guided edit refreshes the JSON in place instead
    // of kicking the user back to the fold view.
    if (state.rawOpen) {
      var raw = body.querySelector("#editor-json-raw");
      var viewEl = body.querySelector("#json-view");
      var toggle = body.querySelector("#editor-json-edit");
      if (raw && viewEl) {
        raw.hidden = false;
        viewEl.hidden = true;
        if (toggle) toggle.classList.add("editor-json__act--on");
      }
    }
    var formEl = body.querySelector("#editor-form");
    var createdPath = formEl && formEl.dataset ? formEl.dataset.focus : null;
    window.HfsEditorAdd.restorePickers(body, state.pickers, createdPath, operation);
    if (window.HfsEditorAdd.revealCreated(body, createdPath, operation)) return;

    var target = state.focus ? inputByPath(state.focus.path) : null;
    if (target) {
      target.focus({ preventScroll: true });
      if (target.setSelectionRange && state.focus.start !== null) {
        try { target.setSelectionRange(state.focus.start, state.focus.end); } catch (ignored) {}
      }
    }

    var tree = body.querySelector(".editor-tree");
    if (tree) tree.scrollTop = state.scroll;
    window.HfsEditorAdd.restoreUndoFocus(body, operation);
  }

  /* The in-flight document. Normally the server's fragment (the hidden field),
     but while the raw editor is open its textarea is the source of truth — the
     field only catches up when you toggle "Edit raw" back off, so a Save typed
     directly in raw mode must read the textarea. */
  function currentDocument() {
    var raw = document.getElementById("editor-json-raw");
    var source = document.getElementById("editor-source");
    if (raw && !raw.hidden && source) return source.value;
    var field = document.getElementById("editor-doc");
    return field ? field.value : "{}";
  }

  function readWithPending() {
    return window.HfsUnsaved.withPending(currentDocument(), body);
  }

  /* ---- loading --------------------------------------------------------- */

  function load() {
    var resource = resourceId
      ? fetch("/" + resourceType + "/" + resourceId, { headers: fhirHeaders() })
          .then(function (response) {
            if (!response.ok) throw new Error(String(response.status));
            return response.json();
          })
      : Promise.resolve({ resourceType: resourceType });
    return resource.then(function (doc) {
      return renderDocument(doc).then(function (rendered) {
        if (!rendered) throw new Error(messages.msgLoadError);
        if (resourceId) confirmIdentity(doc);
        ready = true;
        updateActions();
        trackUnsaved();
        if (unsaved) unsaved.reset();
        loadVersions();
      });
    }).catch(function () { if (root.isConnected) say(messages.msgLoadError, "error"); });
  }

  function renderDocument(resource) {
    window.HfsEditorAdd.invalidateRefresh(body);
    return send("", { doc: JSON.stringify(resource) });
  }

  /* ---- version history panel ------------------------------------------- */

  var versionsHost = document.getElementById("editor-versions-list");

  function loadVersions() {
    if (!versionsHost || !resourceId) return;
    fetch("/" + resourceType + "/" + resourceId + "/_history", {
      headers: fhirHeaders(),
    })
      .then(function (r) { return r.ok ? r.json() : null; })
      .then(function (bundle) {
        if (root.isConnected) renderVersions((bundle && bundle.entry) || []);
      })
      .catch(function () {});
  }

  function renderVersions(entries) {
    versionsHost.textContent = "";
    if (!entries.length) {
      var none = document.createElement("p");
      none.className = "editor-versions__none";
      none.textContent = versionsHost.dataset.msgNone;
      versionsHost.appendChild(none);
      return;
    }
    entries.forEach(function (entry, index) {
      var resource = entry.resource || {};
      var response = entry.response || {};
      var request = entry.request || {};
      var etag = /"([^"]+)"/.exec(response.etag || "");
      var version = (resource.meta && resource.meta.versionId) || (etag && etag[1]) || "";
      var when = (resource.meta && resource.meta.lastUpdated) || response.lastModified || "";
      var method = (request.method || "").toUpperCase();
      var kind =
        method === "POST" ? "create" : method === "PATCH" ? "patch" :
        method === "DELETE" ? "delete" : "update";

      var row = document.createElement("button");
      row.type = "button";
      row.className = "editor-version" + (index === 0 ? " editor-version--current" : "");

      var id = document.createElement("span");
      id.className = "editor-version__id";
      id.textContent = "v" + version;
      var meta = document.createElement("span");
      meta.className = "editor-version__meta";
      meta.textContent =
        (index === 0 ? versionsHost.dataset.msgCurrent : kind) +
        (when ? " · " + new Date(when).toLocaleString() : "");

      row.appendChild(id);
      row.appendChild(meta);
      // Load this version into the editor.
      row.addEventListener("click", function () {
        if (saving) return;
        renderDocument(resource);
        subject.textContent = resourceType + "/" + resourceId + " · v" + version;
      });
      versionsHost.appendChild(row);
    });
  }

  /* ---- raw JSON editing ------------------------------------------------- */

  function applyView() {
    /* No-op retained as the render hook the round trip calls after a swap. */
  }

  root.addEventListener("click", function (event) {
    if (saving && body.contains(event.target)) return;
    /* Raw-edit toggle: swap the fold view for the textarea and back. */
    if (event.target.id === "editor-json-edit") {
      var raw = document.getElementById("editor-json-raw");
      var viewEl = document.getElementById("json-view");
      if (!raw || !viewEl) return;
      if (raw.hidden) {
        raw.hidden = false;
        viewEl.hidden = true;
        event.target.classList.add("editor-json__act--on");
      } else {
        // Leaving raw: the text becomes the document, and the form re-renders.
        // Close the pane before the round trip — the state capture during the
        // swap keeps raw mode alive, and this is the one re-render that must
        // read as "the user left raw mode".
        var source = document.getElementById("editor-source");
        var field = document.getElementById("editor-doc");
        if (source && field) field.value = source.value;
        raw.hidden = true;
        viewEl.hidden = false;
        event.target.classList.remove("editor-json__act--on");
        send("");
      }
      return;
    }

    /* ---- structural mutations: each is one round trip ------------------ */

    var add = event.target.closest("[data-add]");
    if (add) {
      send("add", { path: add.dataset.add, name: add.dataset.name, slice: add.dataset.slice || "" }, window.HfsEditorAdd.operationFrom(add));
      return;
    }

    var remove = event.target.closest("[data-remove]");
    if (remove) {
      var removal = remove.hasAttribute("data-add-undo")
        ? window.HfsEditorAdd.undoOperation(body, remove, currentDocument()) : null;
      if (remove.hasAttribute("data-add-undo") && !removal) return;
      send("remove", { path: remove.dataset.remove }, removal);
      return;
    }

    var extension = event.target.closest("[data-extension]");
    if (extension) {
      var url = window.HfsEditorAdd.extensionUrl(extension);
      send("extension", { path: extension.dataset.extension, url: url }, window.HfsEditorAdd.operationFrom(extension));
      return;
    }

    if (event.target.id === "editor-save") save();
    if (event.target.id === "editor-delete") remove_resource();
  });

  /* A value[x]: the user picks the type, the server creates the branch. */
  root.addEventListener("change", function (event) {
    if (saving) return;
    var choose = event.target.closest("[data-choose]");
    if (choose && choose.value) {
      send("choose", {
        path: choose.dataset.choose,
        name: choose.dataset.declarer,
        arm: choose.value,
      }, window.HfsEditorAdd.operationFrom(choose));
    }
  });

  /* Primitive edits do not round-trip per keystroke -- only on blur, when the
   * value is settled. The server re-validates, so the error appears where the
   * mistake is. */
  root.addEventListener(
    "blur",
    function (event) {
      var input = event.target.closest("[data-set]");
      if (!input) return;
      // An unchanged value needs no round trip -- tabbing through fields
      // must not re-render the panel (#547).
      if (input.value === input.defaultValue) return;
      send("set", { path: input.dataset.set, value: input.value });
    },
    true
  );


  /* Live $expand picker (#365): bound inputs carry data-vs-url; typing
   * debounces a request to the UI's terminology proxy and fills a per-row
   * datalist. 204 (no server configured) leaves the plain input alone. */
  var expandTimer = null;
  var expandSeq = 0;
  var liveListSeq = 0;
  body.addEventListener("input", function (event) {
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
          if (!data || seq !== expandSeq || !input.isConnected) return;
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

  /* The add-picker's own typeahead over the "add" list (#1239). */
  window.HfsEditorAdd.attach(root);

  /* ---- saving ---------------------------------------------------------- */

  /* Every location an issue claims. `location` is the R4 spelling and is
   * deprecated, so it only stands in when `expression` is absent. */
  function expressionsOf(issue) {
    var claimed = issue.expression || issue.location || [];
    return Array.isArray(claimed) ? claimed : [claimed];
  }

  /* The row an OperationOutcome expression names, if we render one.
   *
   * The two sides spell the same path differently: the outcome carries
   * bracket-indexed FHIRPath rooted at the resource type
   * (`Patient.name[0].given`), while rows are keyed on the validator's dotted
   * form (`name.0.given`). Normalise before comparing — a plain === beats
   * building a selector out of server text. */
  function rowFor(expression) {
    var dotted = String(expression).replace(/\[(\d+)\]/g, ".$1");
    var cut = dotted.indexOf(".");
    if (cut <= 0) return null;
    var path = dotted.slice(cut + 1);
    var rows = body.querySelectorAll("[data-path]");
    for (var i = 0; i < rows.length; i++) {
      if (rows[i].dataset.path === path) return rows[i];
    }
    return null;
  }

  /* Adds a message to a row, where the live pass puts its own: under the head,
   * after any error already there. */
  function anchor(row, text) {
    if (!row) return;
    var message = document.createElement("p");
    message.className = "editor-row__error";
    message.textContent = text;
    var existing = row.querySelectorAll(":scope > .editor-row__error");
    var after = existing.length
      ? existing[existing.length - 1]
      : row.querySelector(":scope > .editor-row__head");
    if (after) after.insertAdjacentElement("afterend", message);
    else row.appendChild(message);
    row.classList.add("editor-row--error");
  }

  async function save() {
    if (saving || !ready) return;
    // Commit the focused primitive before capturing the document. Structural
    // edits already reserved by the click share the same mutation queue.
    var authored = rawReplacement && rawReplacement.version === window.HfsEditorAdd.documentVersion(body)
      ? rawReplacement : null;
    var active = document.activeElement;
    if (!canonicalPending && !authored && active && body.contains(active) && active.matches("[data-set]")) active.blur();
    saving = true;
    updateActions();
    var revision = editRevision;
    try {
      // A committed response remains recoverable if its projection failed.
      // Retry projection before any further write, keeping the old view inert.
      if (canonicalPending) {
        if (!await renderDocument(canonicalPending)) { say(messages.msgLoadError, "error"); return; }
        canonicalPending = null;
        if (unsaved) unsaved.reset();
        refreshReturnLinks();
        say("");
        announce(messages.msgSaved);
        return;
      }
      try {
        await window.HfsEditorAdd.whenMutationsSettled(body);
      } catch (error) {
        if (!authored || authored.version !== window.HfsEditorAdd.documentVersion(body) ||
            !window.HfsEditorAdd.supersedeCompletedMutations(body)) throw error;
      }
      if (authored && authored.version !== window.HfsEditorAdd.documentVersion(body)) authored = null;
      if (!root.isConnected || editRevision !== revision) return;
      var doc = authored ? authored.doc : currentDocument();
      var parsed;
      try { parsed = JSON.parse(doc); }
      catch (error) { say(messages.msgSaveInvalid || String(error), "error"); return; }
      window.HfsEditorAdd.invalidateRefresh(body);
      var rendered = await send("", { doc: doc });
      if (!rendered || editRevision !== revision) { say(messages.msgLoadError, "error"); return; }
      var form = body.querySelector("#editor-form");
      var errors = form ? Number(form.dataset.errorCount) : NaN;
      if (!Number.isFinite(errors) || errors > 0) { say(messages.msgSaveBlocked, "error"); return; }
      var isNew = !parsed.id;
      var response = await fetch("/" + resourceType + (isNew ? "" : "/" + encodeURIComponent(parsed.id)), {
        method: isNew ? "POST" : "PUT",
        headers: fhirHeaders({ "Content-Type": "application/fhir+json" }),
        body: doc,
      });
      var payload = await response.json();
      if (!root.isConnected) return;
      if (!response.ok) {
        var issues = (payload && payload.issue) || [];
        issues.forEach(function (issue) {
          var text = issue.diagnostics || (issue.details && issue.details.text) || "";
          expressionsOf(issue).forEach(function (expr) { anchor(rowFor(expr), text); });
        });
        var first = issues[0];
        say((first && (first.diagnostics || (first.details && first.details.text))) || String(response.status), "error");
        return;
      }
      // The response, rather than an authored id, confirms persistence. Install
      // that canonical document before resetting the dirty baseline.
      confirmIdentity(payload);
      canonicalPending = payload;
      if (editRevision !== revision || !await renderDocument(payload)) {
        say(messages.msgLoadError, "error");
        return;
      }
      canonicalPending = null;
      say("");
      announce(messages.msgSaved);
      if (unsaved) unsaved.reset();
      refreshReturnLinks();
      loadVersions();
    } catch (error) {
      if (root.isConnected) say(String(error), "error");
    } finally {
      saving = false;
      updateActions();
    }
  }

  function returnDestination(identity, deleted) {
    var target = new URL(messages.returnTo, window.location.origin);
    if (target.pathname === "/ui/search-parameters" && identity.type === "SearchParameter") {
      target.searchParams.set("refresh", "1");
      if (deleted && identity.url && target.searchParams.get("sel") === identity.url) target.searchParams.delete("sel");
    }
    if (target.pathname === "/ui/compartments" && identity.type === "CompartmentDefinition") {
      target.searchParams.set("refresh", "1");
      if (deleted && identity.code && target.searchParams.get("def") === identity.code) target.searchParams.delete("def");
    }
    return target.pathname + target.search + target.hash;
  }

  function refreshReturnLinks() {
    if (!confirmed) return;
    var href = returnDestination(confirmed, false);
    document.getElementById("editor-back").setAttribute("href", href);
    document.getElementById("editor-cancel").setAttribute("href", href);
  }

  function remove_resource() {
    if (!confirmed || saving) return;
    var identity = confirmed;
    /* The shared in-page confirmation (#1667), not the browser's own box. */
    window.HfsConfirm.ask(messages.msgConfirmDelete, { danger: true }).then(function (ok) {
      if (!ok || saving) return;
      fetch("/" + identity.type + "/" + identity.id, { method: "DELETE", headers: fhirHeaders() })
        .then(function (response) {
          if (!root.isConnected) return;
          if (!response.ok) { say(String(response.status), "error"); return; }
          if (window.HfsUnsaved) window.HfsUnsaved.suspend();
          window.location.href = returnDestination(identity, true);
        })
        .catch(function (error) { if (root.isConnected) say(String(error), "error"); });
    });
  }

  load();
})();
