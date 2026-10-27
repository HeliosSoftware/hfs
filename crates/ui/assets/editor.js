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
 *   2. has the server render the editor body once for that document,
 *   3. mounts the JSON pane and the guided form over it
 *      (`resource-json-editor.js`, which pairs the CodeMirror editor with the
 *      guided form through `editor-pair.js` / `editor-form.js` -- the same
 *      loop the Resources modal and View Definitions use, so every form
 *      interaction and every JSON edit round-trips there, not here),
 *   4. saves the JSON pane's document through the ordinary FHIR API.
 *
 * The body is rendered whole only once per document; afterwards just the
 * guided-form card is swapped and the JSON pane is never replaced (loading a
 * version or installing the saved document goes through the editor as one
 * undoable transaction). The JSON pane's text (`#editor-source`) is the single
 * copy of the document. Nothing here derives from it, so nothing here can lose
 * a key it did not understand -- which is the failure mode of every
 * schema-driven editor we surveyed for #264.
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

  /* Replaced by every full render (`HfsResourceJsonEditor.render`), so always
   * read it through this variable. */
  var body = document.getElementById("editor-body");
  /* `{ view, pair, textarea }` of the mounted JSON pane (`null` pair without
   * editor-pair.js). */
  var session = { view: null, pair: null, textarea: null };
  var JsonEditor = window.HfsResourceJsonEditor;
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
  var deleting = false;
  var ready = false;
  var canonicalPending = null;
  var editRevision = 0;
  var rawReplacement = null;
  /* True while a document is installed into the editor from code (a version,
   * the saved canonical): that is not an edit by the user. */
  var applying = false;
  var saveButton = document.getElementById("editor-save");
  var deleteButton = document.getElementById("editor-delete");
  function updateActions() {
    saveButton.disabled = saving || deleting || !ready;
    if (saving) saveButton.setAttribute("aria-busy", "true");
    else saveButton.removeAttribute("aria-busy");
    deleteButton.hidden = !confirmed;
    deleteButton.disabled = saving || deleting;
    if (deleting) deleteButton.setAttribute("aria-busy", "true");
    else deleteButton.removeAttribute("aria-busy");
    body.inert = saving || deleting || !!canonicalPending;
  }
  /* While nothing is confirmed, the header says where the document will be
   * saved when it carries a valid id (#1751); empty otherwise. */
  function refreshSubject() {
    if (confirmed || resourceId) return;
    var doc;
    try { doc = JSON.parse(currentDocument()); } catch (invalidJson) { doc = null; }
    var text = window.HfsSaveTarget.notice(resourceType, doc, messages.msgSaveTarget || "{target}");
    subject.classList.toggle("subject--target", !!text);
    subject.textContent = "";
    if (!text) return;
    var label = resourceType + "/" + doc.id;
    var at = text.indexOf(label);
    var code = document.createElement("code");
    code.textContent = label;
    subject.appendChild(document.createTextNode(text.slice(0, at)));
    subject.appendChild(code);
    subject.appendChild(document.createTextNode(text.slice(at + label.length)));
  }
  function confirmIdentity(resource) {
    if (!resource || resource.resourceType !== resourceType ||
        !/^[A-Za-z0-9.-]{1,64}$/.test(resource.id || "")) return;
    confirmed = { type: resourceType, id: resource.id, url: resource.url, code: resource.code };
    resourceId = resource.id;
    subject.classList.remove("subject--target");
    subject.textContent = resourceType + "/" + resourceId +
      (resource.meta && resource.meta.lastUpdated
        ? " · " + new Date(resource.meta.lastUpdated).toLocaleString() : "");
    updateActions();
  }
  /* Capture phase: this must run before `editor-pair.js`'s own listener on
   * the plain textarea schedules its sync, so the version it captures already
   * includes this edit. */
  root.addEventListener("input", function (event) {
    if (applying || !event.target.matches("[data-set], #editor-source")) return;
    var isSource = event.target.id === "editor-source";
    if (isSource) refreshSubject();
    var projection = body.querySelector("#editor-pretty");
    // The guided form handing its own projection to the editor is not an
    // edit: the text equals what the server just rendered.
    if (isSource && projection && event.target.value === projection.value) {
      rawReplacement = null;
      return;
    }
    editRevision++;
    window.HfsEditorAdd.invalidateRefresh(body);
    rawReplacement = null;
    if (isSource) {
      var replacement = window.HfsEditorAdd.canonicalDocument(event.target.value);
      var previous = projection ? window.HfsEditorAdd.canonicalDocument(projection.value) : null;
      // Unparseable text is not a replacement; Save reports it normally.
      if (replacement !== null && replacement !== previous) {
        rawReplacement = { doc: event.target.value, version: window.HfsEditorAdd.documentVersion(body) };
      }
    }
  }, true);

  /* Once an explicitly authored replacement is the current projection, later
   * formatting edits must not resurrect a superseded failed mutation (#1751). */
  new MutationObserver(function () {
    if (!rawReplacement || rawReplacement.version !== window.HfsEditorAdd.documentVersion(body)) return;
    var projected = body.querySelector("#editor-doc");
    if (projected && window.HfsEditorAdd.canonicalDocument(projected.value) ===
        window.HfsEditorAdd.canonicalDocument(rawReplacement.doc)) {
      window.HfsEditorAdd.supersedeCompletedMutations(body);
    }
  }).observe(root, { childList: true, subtree: true });
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

  /* ---- the document ---------------------------------------------------- */

  /* The JSON pane's text is the single copy; the guided form follows it. */
  function currentDocument() {
    var source = document.getElementById("editor-source");
    if (source) return source.value;
    var field = document.getElementById("editor-doc");
    return field ? field.value : "{}";
  }

  function readWithPending() {
    return window.HfsUnsaved.withPending(currentDocument(), body);
  }

  /* The whole body, rendered once for `text` and mounted. Also the fallback
   * when editor-pair.js is not there to swap the form in place. */
  function renderFull(text) {
    window.HfsEditorAdd.invalidateRefresh(body);
    return JsonEditor.render(body, text).then(function (rendered) {
      body = rendered.body;
      session = rendered.session;
      if (!session.pair) window.HfsEditorAdd.attach(body);
      updateActions();
      refreshSubject();
      return true;
    }).catch(function (error) { console.debug("Editor render failed", error); return false; });
  }

  /* Puts `text` into the editor as one undoable transaction and refreshes the
   * guided form for it. Resolves true when both show it. */
  function showDocument(text) {
    rawReplacement = null;
    if (!session.pair) return renderFull(text);
    applying = true;
    var shown;
    try { shown = JsonEditor.apply(session, text); } finally { applying = false; }
    return shown.then(function (ok) {
      refreshSubject();
      if (unsaved) unsaved.check();
      return ok;
    });
  }

  /* The validation render a Save runs before writing: the guided form for
   * exactly `text`, without touching the editor. */
  function validateDocument(text) {
    window.HfsEditorAdd.invalidateRefresh(body);
    if (!session.pair) return renderFull(text);
    // A pending JSON -> form sync would supersede this render.
    session.pair.host.beforeMutation();
    return JsonEditor.project(session, text);
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
      return renderFull(JSON.stringify(doc)).then(function (rendered) {
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
        showDocument(JSON.stringify(resource, null, 2));
        subject.textContent = resourceType + "/" + resourceId + " · v" + version;
      });
      versionsHost.appendChild(row);
    });
  }

  root.addEventListener("click", function (event) {
    if (event.target.id === "editor-save") save();
    if (event.target.id === "editor-delete") remove_resource();
  });

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
    if (saving || deleting || !ready) return;
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
        if (!await showDocument(JSON.stringify(canonicalPending, null, 2))) { say(messages.msgLoadError, "error"); return; }
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
      var rendered = await validateDocument(doc);
      if (!rendered || editRevision !== revision) { say(messages.msgLoadError, "error"); return; }
      var form = body.querySelector("#editor-form");
      var errors = form ? Number(form.dataset.errorCount) : NaN;
      if (!Number.isFinite(errors) || errors > 0) { say(messages.msgSaveBlocked, "error"); return; }
      var target = window.HfsSaveTarget.forCreate(resourceType, parsed);
      if (!confirmed && target.method === "PUT" && window.HfsSaveTarget.isValidId(target.id)) {
        // Creating over an id that already exists would silently add a version
        // (#1751). `saving` is already true, so a double click starts no second
        // probe or dialog. A failed probe never blocks the save.
        var exists = false;
        try {
          var probe = await fetch(target.url + "?_elements=id", { method: "GET", headers: fhirHeaders() });
          exists = window.HfsSaveTarget.existsFromStatus(probe.status);
        } catch (probeError) { exists = false; }
        if (exists && !await window.HfsConfirm.ask(
          String(messages.msgIdExists).replace("{target}", resourceType + "/" + target.id),
          { confirmLabel: messages.msgIdExistsConfirm },
        )) return;
        if (!root.isConnected || editRevision !== revision) return;
      }
      var response = await fetch(target.url, {
        method: target.method,
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
      if (editRevision !== revision || !await showDocument(JSON.stringify(payload, null, 2))) {
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
    if (!confirmed || saving || deleting) return;
    var identity = confirmed;
    /* The shared in-page confirmation (#1667), not the browser's own box. */
    window.HfsConfirm.ask(messages.msgConfirmDelete, { danger: true }).then(function (ok) {
      if (!ok || saving || deleting) return;
      deleting = true;
      updateActions();
      fetch("/" + identity.type + "/" + identity.id, { method: "DELETE", headers: fhirHeaders() })
        .then(function (response) {
          if (!root.isConnected) return;
          if (!response.ok) {
            deleting = false;
            updateActions();
            say(String(response.status), "error");
            return;
          }
          // Navigating away: `deleting` stays up so the controls stay inert.
          if (window.HfsUnsaved) window.HfsUnsaved.suspend();
          window.location.href = returnDestination(identity, true);
        })
        .catch(function (error) {
          if (!root.isConnected) return;
          deleting = false;
          updateActions();
          say(String(error), "error");
        });
    });
  }

  load();
})();
