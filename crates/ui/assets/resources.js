/*
 * Resources workspace (#282): the Edit Resource modal, and "Create new".
 *
 * The search, the type rail, and the results table are the same components the
 * Search page uses (search-builder.js), so this script owns only the modal: it
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
  /* Replaced by every full render (`HfsResourceJsonEditor.render`) and on
   * close, so always read it through this variable. */
  var editorBody = document.getElementById("resource-editor-body");
  /* `{ view, pair, textarea }` of the mounted JSON pane (`null` pair without
   * editor-pair.js). */
  var editor = { view: null, pair: null, textarea: null };
  var JsonEditor = window.HfsResourceJsonEditor;

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

  /* The body element is swapped on every render, so listen on the modal. */
  modal.addEventListener("input", function (event) {
    if (event.target.id === "editor-source") refreshSubject();
  });

  /* Pending edits (#1240): a guided-form `[data-set]` control only
   * round-trips through the guided-form loop (editor-form.js) on blur, so
   * the JSON pane alone lags a keystroke behind what is actually on screen.
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
   * lands, the editor's own mutation events re-check it on every swap. */
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
    // Drop the editor with its listeners; the next open mounts a fresh one.
    resetEditorBody();
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

  /* The editor inside the modal is the same fragment the Editor page uses,
   * mounted by the shared `resource-json-editor.js`: the JSON pane is the
   * code editor and `editor-pair.js` / `editor-form.js` run every form
   * interaction. This script only renders the body for a document and reads
   * the document back for Save. */
  var renderSeq = 0;

  /* An empty body element in place of the current one, so no listener of the
   * mounted editor outlives it. */
  function resetEditorBody() {
    renderSeq++;
    JsonEditor.destroy(editorBody);
    var fresh = document.createElement(editorBody.tagName);
    fresh.id = editorBody.id;
    fresh.className = editorBody.className;
    editorBody.replaceWith(fresh);
    editorBody = fresh;
    editor = { view: null, pair: null, textarea: null };
  }

  /* Resolves true once rendered and mounted, false on failure, undefined when
   * a newer render superseded this one. */
  function renderFull(text) {
    var seq = ++renderSeq;
    window.HfsEditorAdd.invalidateRefresh(editorBody);
    return JsonEditor.render(editorBody, text, function () { return seq !== renderSeq; })
      .then(function (rendered) {
        if (!rendered) return undefined;
        editorBody = rendered.body;
        editor = rendered.session;
        if (!editor.pair) window.HfsEditorAdd.attach(editorBody);
        refreshSubject();
        return true;
      })
      .catch(function (error) { console.debug("Resource editor render failed", error); return false; });
  }

  function renderEditor(resource) {
    return renderFull(JSON.stringify(resource));
  }

  /* Installs the saved canonical document into the editor as one undoable
   * transaction and refreshes the form (a full render without editor-pair.js). */
  function showDocument(text) {
    if (!editor.pair) return renderFull(text);
    return JsonEditor.apply(editor, text).then(function (ok) { refreshSubject(); return ok; });
  }

  /* The validation render Save runs: the guided form for exactly `text`. */
  function validateDocument(text) {
    window.HfsEditorAdd.invalidateRefresh(editorBody);
    if (!editor.pair) return renderFull(text);
    editor.pair.host.beforeMutation();
    return JsonEditor.project(editor, text);
  }

  function openResource(type, id) {
    current = { type: type, id: id };
    setSubject(type + "/" + id);
    openModal();
    editorBody.innerHTML = "";
    fetch("/" + type + "/" + id, { headers: fhirHeaders() })
      .then(function (r) { if (!r.ok) throw new Error(String(r.status)); return r.json(); })
      .then(renderEditor)
      .then(function (ok) {
        if (ok === false) say(messages.msgLoadError, "error");
        if (unsaved) unsaved.reset();
      })
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
   * trusted resource identity attached by search-builder.js instead of parsing
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

  // The JSON pane's text is the single copy of the document.
  function currentDocText() {
    var source = editorBody.querySelector("#editor-source");
    if (source) return source.value;
    var field = editorBody.querySelector("#editor-doc");
    return field ? field.value : "{}";
  }

  function currentDoc() {
    try { return JSON.parse(currentDocText()); } catch (e) { return null; }
  }

  /* Creating with an id that already belongs to a resource would silently add
   * a version to it (#1751): probe first and ask. Resolves true to go on with
   * the write. A failed probe never blocks the save. Runs inside Save's busy
   * window, so a double click cannot start a second probe or dialog. */
  function confirmCreateOverExisting(target) {
    if (current.id || target.method !== "PUT" || !window.HfsSaveTarget.isValidId(target.id)) {
      return Promise.resolve(true);
    }
    return fetch(target.url + "?_elements=id", { method: "GET", headers: fhirHeaders() })
      .then(function (r) { return window.HfsSaveTarget.existsFromStatus(r.status); })
      .catch(function () { return false; })
      .then(function (exists) {
        if (!exists) return true;
        var label = current.type + "/" + target.id;
        return window.HfsConfirm.ask(
          String(messages.msgIdExists).replace("{target}", label),
          { confirmLabel: messages.msgIdExistsConfirm },
        );
      });
  }

  document.getElementById("resource-save").addEventListener("click", function () {
    // The whole chain runs under the shared busy state (#1750): repeat clicks
    // are dropped until it settles, whatever the outcome.
    var saveButton = document.getElementById("resource-save");
    var deleteButton = document.getElementById("resource-delete");
    window.hfsBusy.during([saveButton], function () {
      // A guided-form field commits on blur, which the click on Save itself
      // causes: let that round trip land in the JSON pane before reading it.
      return window.HfsEditorAdd.whenMutationsSettled(editorBody).then(function () {
        var doc = currentDoc();
        if (!doc) { say(messages.msgSaveInvalid, "error"); return; }
        // Validate the exact document being saved by re-rendering the form
        // for it, then block the save if the editor reports any issue — an
        // invalid resource must not be persisted.
        return validateDocument(JSON.stringify(doc)).then(function (rendered) {
          if (!rendered) { say(messages.msgLoadError, "error"); return; }
          var form = editorBody.querySelector("#editor-form");
          var errors = form ? parseInt(form.dataset.errorCount || "0", 10) : 0;
          if (errors > 0) { say(messages.msgSaveBlocked, "error"); return; }
          var target = current.id
            ? { method: "PUT", url: "/" + current.type + "/" + current.id }
            : window.HfsSaveTarget.forCreate(current.type, doc);
          return confirmCreateOverExisting(target).then(function (go) {
            if (!go) return;
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
                return showDocument(JSON.stringify(res.body, null, 2)).then(function () { if (unsaved) unsaved.reset(); });
              })
              .catch(function () { say(messages.msgLoadError, "error"); });
          });
        });
      }, function () { say(messages.msgLoadError, "error"); });
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
