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

  /* Each renderDiff() call takes a ticket; a response only lands if its
   * ticket is still the newest. Two quick selections (a `change` on each
   * select, say) start two fetches, and the server does not answer them in
   * order — the first one's HTML arriving last would leave the diff showing
   * a comparison nobody is asking for any more. */
  var diffTicket = 0;

  function renderDiff() {
    var fromIndex = Number(fromSel.value);
    var toIndex = Number(toSel.value);
    var from = versions[fromIndex];
    var to = versions[toIndex];
    if (!from || !to) return;

    markSelected(fromIndex, toIndex);
    var ticket = ++diffTicket;

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
        if (ticket !== diffTicket) return;
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
    if (type && id) {
      if (activeScope() !== "instance") selectTab(tabs[0], false);
      load(type, id);
    } else if (activeScope() === "type") {
      loadFeed("type", null);
    }
  });

  /* ---- Type Feed / System Feed tabs (#1674) ---------------------------- */

  var tabs = Array.prototype.slice.call(root.querySelectorAll('[role="tab"]'));
  var instancePanel = document.getElementById("history-panel-instance");
  var feedPanel = document.getElementById("history-panel-feed");
  var feedPath = document.getElementById("history-feed-path");
  var feedRows = document.getElementById("history-feed-rows");
  var feedStatus = document.getElementById("history-feed-status");
  var feedMore = document.getElementById("history-feed-more");
  var FEED_PAGE = 20;
  var feedTicket = 0;
  var feedScope = "";
  var feedNext = null;

  function activeScope() {
    var on = tabs.filter(function (tab) {
      return tab.getAttribute("aria-selected") === "true";
    })[0];
    return on ? on.dataset.tab : "instance";
  }

  function selectTab(tab, focus) {
    tabs.forEach(function (other) {
      var on = other === tab;
      other.setAttribute("aria-selected", on ? "true" : "false");
      other.classList.toggle("tab--on", on);
      other.tabIndex = on ? 0 : -1;
    });
    if (focus) tab.focus();
    var scope = tab.dataset.tab;
    instancePanel.hidden = scope !== "instance";
    feedPanel.hidden = scope === "instance";
    if (scope !== "instance") loadFeed(scope, null);
  }

  function feedBase(scope) {
    if (scope === "system") return "/_history";
    var type = locateForm.elements.type.value.trim() || "Patient";
    return "/" + encodeURIComponent(type) + "/_history";
  }

  /* A next link points at the server's public base URL; the browser follows
   * it on its own origin, like every other FHIR call on this page. */
  function nextLink(bundle) {
    var links = (bundle && bundle.link) || [];
    for (var i = 0; i < links.length; i++) {
      if (links[i].relation === "next" && links[i].url) {
        try {
          var url = new URL(links[i].url, window.location.href);
          return url.pathname + url.search;
        } catch (e) {
          return null;
        }
      }
    }
    return null;
  }

  function loadFeed(scope, url) {
    var ticket = ++feedTicket;
    if (!url) {
      feedScope = scope;
      feedRows.textContent = "";
      feedPath.textContent = feedBase(scope);
      url = feedBase(scope) + "?_count=" + FEED_PAGE;
    }
    feedStatus.hidden = true;
    feedMore.hidden = true;
    fetch(url, { headers: fhirHeaders() })
      .then(function (response) {
        if (!response.ok) throw new Error(String(response.status));
        return response.json();
      })
      .then(function (bundle) {
        if (ticket !== feedTicket) return;
        ((bundle && bundle.entry) || []).forEach(function (entry) {
          feedRows.appendChild(feedRow(entry));
        });
        feedNext = nextLink(bundle);
        feedMore.hidden = !feedNext;
        if (!feedRows.children.length) {
          feedStatus.textContent = messages.msgFeedEmpty;
          feedStatus.hidden = false;
        }
      })
      .catch(function () {
        if (ticket !== feedTicket) return;
        feedStatus.textContent = messages.msgFeedError;
        feedStatus.hidden = false;
      });
  }

  function cell(child) {
    var td = document.createElement("td");
    if (typeof child === "string") td.textContent = child;
    else td.appendChild(child);
    return td;
  }

  /* One history entry: which resource, which version, what produced it, when.
   * A deletion carries no resource, so its type/id come from request.url. */
  function feedRow(entry) {
    var resource = entry.resource || {};
    var request = entry.request || {};
    var response = entry.response || {};
    var type = resource.resourceType || "";
    var id = resource.id || "";
    if ((!type || !id) && request.url) {
      var parts = request.url.split("?")[0].split("/");
      type = type || parts[0] || "";
      id = id || parts[1] || "";
    }
    var meta = resource.meta || {};
    var row = document.createElement("tr");
    var label = type + (id ? "/" + id : "");
    if (type && id) {
      var link = document.createElement("a");
      link.href =
        "/ui/history?type=" + encodeURIComponent(type) + "&id=" + encodeURIComponent(id);
      link.textContent = label;
      row.appendChild(cell(link));
    } else {
      row.appendChild(cell(label));
    }
    var version = meta.versionId || etagVersion(response.etag) || "";
    row.appendChild(cell(version ? "v" + version : ""));
    row.appendChild(cell(labelFor((request.method || "").toUpperCase())));
    row.appendChild(cell(shortTime(meta.lastUpdated || response.lastModified || "")));
    return row;
  }

  tabs.forEach(function (tab) {
    tab.addEventListener("click", function () {
      selectTab(tab, false);
    });
    tab.addEventListener("keydown", function (event) {
      var index = tabs.indexOf(tab);
      var next = null;
      if (event.key === "ArrowRight") next = tabs[(index + 1) % tabs.length];
      else if (event.key === "ArrowLeft") next = tabs[(index - 1 + tabs.length) % tabs.length];
      else if (event.key === "Home") next = tabs[0];
      else if (event.key === "End") next = tabs[tabs.length - 1];
      if (!next) return;
      event.preventDefault();
      selectTab(next, true);
    });
  });

  feedMore.addEventListener("click", function () {
    if (feedNext) loadFeed(feedScope, feedNext);
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
