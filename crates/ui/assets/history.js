/*
 * History & Versions (#236).
 *
 * Thin, like the editor: the interesting work — the two-layer diff — happens in
 * Rust, behind /ui/history/diff. This script fetches the version list and the
 * two selected versions from the ordinary FHIR _history / vread API, then posts
 * them to be rendered. The diff is never computed in the browser.
 *
 * The version list fetch and vread are plain reads the storage layer already
 * serves; nothing here is a new endpoint.
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

  var root = document.getElementById("history");
  if (!root || !window.fetch) return;

  var messages = root.dataset;
  var pathEl = document.getElementById("history-path");
  var versionsEl = document.getElementById("history-versions");
  var controls = document.getElementById("history-controls");
  var fromSel = document.getElementById("history-from");
  var toSel = document.getElementById("history-to");
  var metaToggle = document.getElementById("history-show-metadata");
  var diffEl = document.getElementById("history-diff");
  var locateForm = document.getElementById("history-locate");

  // Each entry: { versionId, lastUpdated, method, deleted, resource }
  var versions = [];
  var resourceType = "";
  var resourceId = "";

  /* ---- load the history feed ------------------------------------------- */

  function load(type, id) {
    resourceType = type;
    resourceId = id;
    pathEl.textContent = "/" + type + "/" + id + "/_history";

    fetch("/" + type + "/" + id + "/_history", {
      headers: fhirHeaders(),
    })
      .then(function (response) {
        if (response.status === 404) throw new Error("not-found");
        if (!response.ok) throw new Error(String(response.status));
        return response.json();
      })
      .then(function (bundle) {
        versions = parseBundle(bundle);
        if (!versions.length) throw new Error("not-found");
        render();
      })
      .catch(function (error) {
        versionsEl.textContent = "";
        controls.hidden = true;
        diffEl.innerHTML =
          '<p class="history__empty">' +
          (error.message === "not-found" ? messages.msgNotFound : messages.msgLoadError) +
          "</p>";
      });
  }

  /* Pulls the versions out of a history Bundle, newest first. The interaction
   * that produced each version is in entry.request.method (POST=create,
   * PUT=update, PATCH=patch, DELETE=delete) — the same signal Brett's rail
   * shows. The version id and timestamp come from entry.response (etag /
   * lastModified), which is where this server carries them; resource.meta is a
   * fallback for servers that stamp it onto the resource instead. */
  function parseBundle(bundle) {
    var entries = (bundle && bundle.entry) || [];
    return entries.map(function (entry) {
      var resource = entry.resource || {};
      var meta = resource.meta || {};
      var request = entry.request || {};
      var response = entry.response || {};
      var method = (request.method || "").toUpperCase();
      var status = response.status || "";
      var deleted = method === "DELETE" || /^410/.test(status);
      return {
        versionId: meta.versionId || etagVersion(response.etag) || "",
        lastUpdated: meta.lastUpdated || response.lastModified || "",
        method: labelFor(method),
        deleted: deleted,
        resource: resource,
      };
    });
  }

  /* W/"3" -> "3" */
  function etagVersion(etag) {
    if (!etag) return "";
    var match = /"([^"]+)"/.exec(etag);
    return match ? match[1] : "";
  }

  function labelFor(method) {
    if (method === "POST") return "create";
    if (method === "PUT") return "update";
    if (method === "PATCH") return "patch";
    if (method === "DELETE") return "delete";
    return "version";
  }

  /* ---- render the rail and default comparison -------------------------- */

  function render() {
    versionsEl.textContent = "";
    fromSel.textContent = "";
    toSel.textContent = "";

    versions.forEach(function (version, index) {
      var row = document.createElement("button");
      row.type = "button";
      row.className = "history-version" + (index === 0 ? " history-version--current" : "");
      row.dataset.index = String(index);

      var id = document.createElement("span");
      id.className = "history-version__id";
      id.textContent = "v" + version.versionId;
      var when = document.createElement("span");
      when.className = "history-version__when";
      when.textContent = shortTime(version.lastUpdated);
      var kind = document.createElement("span");
      kind.className = "history-version__kind history-version__kind--" + version.method;
      kind.textContent = index === 0 ? messages.msgCurrent : version.method;

      row.appendChild(id);
      row.appendChild(when);
      row.appendChild(kind);
      versionsEl.appendChild(row);

      fromSel.appendChild(option(index, "v" + version.versionId));
      toSel.appendChild(option(index, "v" + version.versionId));
    });

    // Default: newest vs the one before it — the adjacent comparison the
    // decision doc chose.
    if (versions.length >= 2) {
      fromSel.value = "1";
      toSel.value = "0";
    } else {
      fromSel.value = "0";
      toSel.value = "0";
    }
    controls.hidden = versions.length < 1;
    renderDiff();
  }

  function option(value, label) {
    var el = document.createElement("option");
    el.value = String(value);
    el.textContent = label;
    return el;
  }

  /* ---- post the two versions to be diffed ------------------------------ */

  function renderDiff() {
    var fromIndex = Number(fromSel.value);
    var toIndex = Number(toSel.value);
    var from = versions[fromIndex];
    var to = versions[toIndex];
    if (!from || !to) return;

    markSelected(fromIndex, toIndex);

    var body = new URLSearchParams();
    body.set("from", JSON.stringify(from.resource));
    body.set("to", JSON.stringify(to.resource));
    body.set("from_label", "v" + from.versionId);
    body.set("to_label", "v" + to.versionId);
    body.set("show_metadata", metaToggle.checked ? "true" : "false");
    body.set("deleted", to.deleted ? "true" : "false");

    fetch("/ui/history/diff", { method: "POST", body: body })
      .then(function (response) {
        return response.text();
      })
      .then(function (html) {
        diffEl.innerHTML = html;
      });
  }

  function markSelected(fromIndex, toIndex) {
    versionsEl.querySelectorAll(".history-version").forEach(function (row) {
      var index = Number(row.dataset.index);
      row.classList.toggle("history-version--from", index === fromIndex);
      row.classList.toggle("history-version--to", index === toIndex);
    });
  }

  /* ---- interactions ---------------------------------------------------- */

  fromSel.addEventListener("change", renderDiff);
  toSel.addEventListener("change", renderDiff);
  metaToggle.addEventListener("change", renderDiff);

  // Clicking a version in the rail sets it as the "to" side and compares it
  // against the version before it.
  versionsEl.addEventListener("click", function (event) {
    var row = event.target.closest(".history-version");
    if (!row) return;
    var index = Number(row.dataset.index);
    toSel.value = String(index);
    fromSel.value = String(Math.min(index + 1, versions.length - 1));
    renderDiff();
  });

  locateForm.addEventListener("submit", function (event) {
    event.preventDefault();
    var type = locateForm.elements.type.value.trim();
    var id = locateForm.elements.id.value.trim();
    if (type && id) load(type, id);
  });

  function shortTime(iso) {
    if (!iso) return "";
    var date = new Date(iso);
    return isNaN(date) ? iso : date.toLocaleString();
  }

  /* Deep link: /ui/history?type=Patient&id=a12 loads straight away. */
  var params = new URLSearchParams(window.location.search);
  var linkType = params.get("type");
  var linkId = params.get("id");
  if (linkType && linkId) {
    locateForm.elements.type.value = linkType;
    locateForm.elements.id.value = linkId;
    load(linkType, linkId);
  }
})();
