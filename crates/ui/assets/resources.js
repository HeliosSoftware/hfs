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
    var form = new URLSearchParams();
    form.set("doc", JSON.stringify(resource));
    form.set("op", "");
    return fetch("/ui/editor/render", { method: "POST", body: form })
      .then(function (r) { return r.text(); })
      .then(function (html) { editorBody.innerHTML = html; });
  }

  function openResource(type, id) {
    current = { type: type, id: id };
    subject.textContent = type + "/" + id;
    openModal();
    editorBody.innerHTML = "";
    fetch("/" + type + "/" + id, { headers: { Accept: "application/fhir+json" } })
      .then(function (r) { if (!r.ok) throw new Error(String(r.status)); return r.json(); })
      .then(renderEditor)
      .catch(function () { say(messages.msgLoadError, "error"); });
  }

  function openNew(type) {
    current = { type: type, id: "" };
    subject.textContent = type + " · " + "new";
    openModal();
    renderEditor({ resourceType: type });
  }

  /* Clicking a result row opens it. The results table (from saved-queries.js)
   * renders id links as `/{type}/{id}`; intercept them into the modal. */
  root.addEventListener(
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
    var field = editorBody.querySelector("#editor-doc");
    if (!field) return null;
    try { return JSON.parse(field.value); } catch (e) { return null; }
  }

  document.getElementById("resource-save").addEventListener("click", function () {
    var doc = currentDoc();
    if (!doc) { say(messages.msgLoadError, "error"); return; }
    var creating = !current.id;
    var url = creating ? "/" + current.type : "/" + current.type + "/" + current.id;
    fetch(url, {
      method: creating ? "POST" : "PUT",
      headers: { "Content-Type": "application/fhir+json", Accept: "application/fhir+json" },
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

  document.getElementById("resource-delete").addEventListener("click", function () {
    if (!current.id) { closeModal(); return; }
    if (!window.confirm(messages.msgConfirmDelete)) return;
    fetch("/" + current.type + "/" + current.id, { method: "DELETE" })
      .then(function (r) {
        if (r.ok || r.status === 204) { closeModal(); location.reload(); }
        else say(String(r.status), "error");
      })
      .catch(function () { say(messages.msgLoadError, "error"); });
  });

  /* ---- history tab: version rail + diff (#236) ------------------------- */

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
      headers: { Accept: "application/fhir+json" },
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
      row.textContent = "v" + v.versionId + (i === 0 ? " · " + messages.msgCurrent : "");
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
