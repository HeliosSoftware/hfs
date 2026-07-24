/*
 * Saved FHIR queries & search builder (issue #234), backed by the per-user
 * settings document and the FHIR REST API itself.
 *
 * The page shell is server-rendered; this script owns three client concerns:
 *
 * 1. The read/modify/write cycle against /_user/settings. Unlike theme.js (a
 *    single last-write-wins scalar), saved queries are structural state
 *    shared across tabs and devices, so every write is a JSON merge patch
 *    conditional on the document's ETag, retried once against a fresh read
 *    when another writer won the race (412).
 * 2. The visual builder: condition / include / result-control rows kept in
 *    two-way sync with the GET URL. Parameter suggestions come from
 *    /ui/queries/params — a server-rendered datalist fragment fed by the
 *    SearchParameter registry, swapped per resource type.
 * 3. Results: Run fetches the search from the FHIR API (the existing REST
 *    surface, no UI-facing endpoint) and renders Bundle.total, a table whose
 *    columns honor _elements, and paging over Bundle.link.
 *
 * Document conventions (see helios-persistence's user_settings module docs):
 * - savedQueries.<ResourceType>.<id> = { name, query, createdAt,
 *   lastAccessedAt?, accessCount? }. Keyed by id precisely so a merge patch
 *   can touch one entry without clobbering siblings.
 * - recentSearches = [{ query: "/Patient?name=smith", at: ISO }] — newest
 *   first, deduped by query, capped. An array (replaced wholesale by every
 *   merge patch) is fine here: it is a small bounded cache rewritten on each
 *   run, not sibling-keyed state.
 */
(function () {
  "use strict";

  var SETTINGS = "/_user/settings";
  /* The effective tenant, stamped by the server (#344); FHIR calls carry it. */
  var TENANT = (document.querySelector('meta[name="hfs-tenant"]') || {}).content || "";
  function fhirHeaders() {
    var h = { Accept: "application/fhir+json" };
    if (TENANT) h["X-Tenant-ID"] = TENANT;
    return h;
  }
  var MAX_RETRIES = 2;
  var MAX_RECENT = 10;

  /* The builder, the results table and the recent list are shared with the
   * Search page (#255), which renders the same partials; the saved-query list
   * is this page's alone. So the script keys off the form, and treats the list
   * host as optional. Prose comes from the shared message carrier either page
   * renders. */
  var root = document.getElementById("saved-queries");
  var messageHost = document.getElementById("search-messages");
  var errorHost = document.getElementById("search-error");
  var form = document.getElementById("saved-query-form");
  var recentHost = document.getElementById("recent-searches");
  var sections = document.getElementById("builder-sections");
  var urlInput = form && form.elements.url;
  if (!form || !messageHost || !window.fetch) return;

  var messages = messageHost.dataset;
  var etag = null;
  var lang = document.documentElement.lang || undefined;

  function fetchDocument() {
    return fetch(SETTINGS, {
      headers: { Accept: "application/json" },
      credentials: "same-origin",
    }).then(function (response) {
      if (response.status === 501) throw { unavailable: true };
      if (!response.ok) throw new Error("settings fetch failed");
      etag = response.headers.get("ETag");
      return response.json();
    });
  }

  /* Merge-patches the document, conditional on the last ETag we saw; on 412
   * re-reads and retries so an unrelated concurrent write (another tab, the
   * theme toggle) never surfaces to the user. */
  function patchDocument(patch, attempt) {
    var headers = { "Content-Type": "application/json" };
    if (etag) headers["If-Match"] = etag;

    return fetch(SETTINGS, {
      method: "PATCH",
      headers: headers,
      credentials: "same-origin",
      body: JSON.stringify(patch),
    }).then(function (response) {
      if (response.status === 412 && attempt < MAX_RETRIES) {
        return fetchDocument().then(function () {
          return patchDocument(patch, attempt + 1);
        });
      }
      if (!response.ok) {
        return response
          .json()
          .catch(function () {
            return null;
          })
          .then(function (outcome) {
            throw { outcome: outcome };
          });
      }
      etag = response.headers.get("ETag");
      return response.json();
    });
  }

  /* Scopes a merge patch to one saved-query entry (or null to delete it). */
  function patchEntry(resourceType, id, entry) {
    var patch = { savedQueries: {} };
    patch.savedQueries[resourceType] = {};
    patch.savedQueries[resourceType][id] = entry;
    return patchDocument(patch, 0);
  }

  function savedQueries(doc) {
    var byType = doc && doc.savedQueries;
    return byType && typeof byType === "object" && !Array.isArray(byType)
      ? byType
      : {};
  }

  function recentSearches(doc) {
    var list = doc && doc.recentSearches;
    if (!Array.isArray(list)) return [];
    return list.filter(function (item) {
      return item && typeof item.query === "string";
    });
  }

  /* Accepts "GET /Patient?name=smith", "/Patient?...", or an absolute URL;
   * the resource type comes from the path. Returns null when it cannot. */
  function parseSearchUrl(raw) {
    var text = (raw || "").trim().replace(/^GET\s+/i, "");
    if (/^https?:\/\//i.test(text)) {
      try {
        var url = new URL(text);
        text = url.pathname + url.search;
      } catch (e) {
        return null;
      }
    }
    var match = /^\/?([A-Za-z]+)(?:\?(.*))?$/.exec(text);
    if (!match) return null;
    return { type: match[1], query: (match[2] || "").trim() };
  }

  function searchPath(resourceType, query) {
    return (
      "/" + encodeURIComponent(resourceType) + (query ? "?" + query : "")
    );
  }

  /* ---- Visual builder: rows kept in two-way sync with the GET URL ------ */

  var CONTROL_KEYS = ["_count", "_sort", "_total", "_summary", "_elements"];
  var INCLUDE_KEYS = ["_include", "_revinclude"];
  var COLON_MODIFIERS = [
    "exact", "contains", "missing", "not", "text",
    "above", "below", "in", "not-in", "identifier", "of-type",
  ];
  /* Comparator prefixes live on the value (ge2020-01-01), not the key. */
  var PREFIXES = ["eq", "ne", "gt", "ge", "lt", "le", "sa", "eb", "ap"];
  var PREFIX_RE = /^(eq|ne|gt|ge|lt|le|sa|eb|ap)(?=[\d])/;
  var catalogType = null;

  /* Swaps the parameter datalist for the current resource type. The
   * fragment is server-rendered from the SearchParameter registry. */
  function loadCatalog(type) {
    if (!type || type === catalogType) return;
    catalogType = type;
    fetch("/ui/queries/params?type=" + encodeURIComponent(type), {
      credentials: "same-origin",
    })
      .then(function (response) {
        return response.ok ? response.text() : null;
      })
      .then(function (html) {
        if (!html) return;
        var tpl = document.createElement("template");
        tpl.innerHTML = html;
        var next = tpl.content.querySelector("datalist");
        var current = document.getElementById("param-options");
        if (next && current) current.replaceWith(next);
      });
  }

  function splitQuery(query) {
    return (query || "")
      .split("&")
      .filter(Boolean)
      .map(function (pair) {
        var eq = pair.indexOf("=");
        var rawKey = eq < 0 ? pair : pair.slice(0, eq);
        var value = eq < 0 ? "" : pair.slice(eq + 1);
        try {
          value = decodeURIComponent(value.replace(/\+/g, " "));
        } catch (e) {
          /* keep the raw value */
        }
        var colon = rawKey.indexOf(":");
        return {
          key: colon < 0 ? rawKey : rawKey.slice(0, colon),
          modifier: colon < 0 ? "" : rawKey.slice(colon + 1),
          value: value,
        };
      });
  }

  function bucketFor(part) {
    if (CONTROL_KEYS.indexOf(part.key) >= 0) return "control";
    if (INCLUDE_KEYS.indexOf(part.key) >= 0) return "include";
    return "condition";
  }

  function option(select, value, label, selected) {
    var el = document.createElement("option");
    el.value = value;
    el.textContent = label;
    el.selected = selected;
    select.appendChild(el);
  }

  function builderRow(kind, part) {
    var row = document.createElement("div");
    row.className = "builder-row";

    var key;
    if (kind === "condition") {
      key = document.createElement("input");
      key.value = part.key;
      key.setAttribute("list", "param-options");
      key.placeholder = sections.dataset.msgParam;
      key.spellcheck = false;
    } else {
      key = document.createElement("select");
      var keys = kind === "include" ? INCLUDE_KEYS : CONTROL_KEYS;
      keys.forEach(function (k) {
        option(key, k, k, part.key === k);
      });
    }
    key.className = "builder-row__key";
    row.appendChild(key);

    /* Comparator prefixes render in the modifier select but are rejoined
     * onto the value; colon modifiers are rejoined onto the key. */
    var value = part.value;
    var selectedMod = part.modifier;
    var prefix = PREFIX_RE.exec(value);
    if (!selectedMod && prefix) {
      selectedMod = prefix[1];
      value = value.slice(2);
    }

    if (kind !== "control") {
      var modifier = document.createElement("select");
      modifier.className = "builder-row__modifier";
      option(modifier, "", "—", !selectedMod);
      if (kind === "include") {
        option(modifier, "iterate", ":iterate", selectedMod === "iterate");
      } else {
        COLON_MODIFIERS.forEach(function (m) {
          option(modifier, m, ":" + m, selectedMod === m);
        });
        PREFIXES.forEach(function (p) {
          option(modifier, p, p, selectedMod === p);
        });
      }
      row.appendChild(modifier);
    }

    var valueInput = document.createElement("input");
    valueInput.className = "builder-row__value";
    valueInput.value = value;
    valueInput.placeholder = sections.dataset.msgValue;
    valueInput.spellcheck = false;
    row.appendChild(valueInput);

    var remove = document.createElement("button");
    remove.type = "button";
    remove.className = "builder-row__remove";
    remove.dataset.removeRow = "true";
    remove.setAttribute("aria-label", sections.dataset.msgRemove);
    remove.textContent = "×";
    row.appendChild(remove);

    return row;
  }

  function builderHosts() {
    return {
      condition: document.getElementById("builder-conditions"),
      include: document.getElementById("builder-includes"),
      control: document.getElementById("builder-controls"),
    };
  }

  /* Marks the picker rail's active type. */
  function markRailType(type) {
    document.querySelectorAll("[data-rail-type]").forEach(function (item) {
      if (item.dataset.railType === type) {
        item.setAttribute("aria-current", "true");
      } else {
        item.removeAttribute("aria-current");
      }
    });
  }

  /* URL → rows. */
  function renderBuilder() {
    if (!sections || !urlInput) return;
    var parsed = parseSearchUrl(urlInput.value);
    if (!parsed) {
      sections.hidden = true;
      markRailType(null);
      return;
    }
    sections.hidden = false;
    sections.dataset.type = parsed.type;
    markRailType(parsed.type);
    loadCatalog(parsed.type);

    var hosts = builderHosts();
    Object.keys(hosts).forEach(function (kind) {
      hosts[kind].textContent = "";
    });
    splitQuery(parsed.query).forEach(function (part) {
      var kind = bucketFor(part);
      hosts[kind].appendChild(builderRow(kind, part));
    });
  }

  /* Rows → URL. */
  function updateUrl() {
    if (!sections || !urlInput) return;
    var type = sections.dataset.type || "";
    var parts = [];
    sections.querySelectorAll(".builder-row").forEach(function (row) {
      var key = row.querySelector(".builder-row__key").value.trim();
      if (!key) return;
      var modifierEl = row.querySelector(".builder-row__modifier");
      var mod = modifierEl ? modifierEl.value : "";
      var value = row.querySelector(".builder-row__value").value.trim();
      if (PREFIXES.indexOf(mod) >= 0) value = mod + value;
      else if (mod) key += ":" + mod;
      parts.push(key + "=" + value);
    });
    urlInput.value =
      "GET /" + type + (parts.length ? "?" + parts.join("&") : "");
  }

  if (sections && urlInput) {
    urlInput.addEventListener("change", renderBuilder);
    sections.addEventListener("input", function (event) {
      if (event.target.closest(".builder-row")) updateUrl();
    });
    sections.addEventListener("click", function (event) {
      var remove = event.target.closest("[data-remove-row]");
      var add = event.target.closest("[data-add]");
      if (remove) {
        remove.closest(".builder-row").remove();
        updateUrl();
      } else if (add) {
        var kind = add.dataset.add;
        var part = {
          key: kind === "include" ? "_include" : kind === "control" ? "_count" : "",
          modifier: "",
          value: "",
        };
        var row = builderRow(kind, part);
        builderHosts()[kind].appendChild(row);
        row.querySelector(kind === "condition" ? ".builder-row__key" : ".builder-row__value").focus();
      }
    });
  }

  /* ---- Resource picker rail --------------------------------------------
   * The type list is server-rendered from the spec; counts hydrate here
   * via the standard `_summary=count` search (Bundle.total only, no
   * entries), a few at a time so 145 types don't stampede the server. */

  var railList = document.getElementById("type-rail-list");
  var railFilter = document.getElementById("type-rail-filter");
  var countFormat = new Intl.NumberFormat(lang);

  function hydrateCounts() {
    if (!railList) return;
    var pending = Array.prototype.slice.call(
      railList.querySelectorAll("[data-count-for]")
    );
    var CONCURRENCY = 4;

    function next() {
      var slot = pending.shift();
      if (!slot) return;
      /* _total=accurate as well: this server only computes Bundle.total
       * when asked explicitly, even under _summary=count. */
      fetch(
        "/" +
          encodeURIComponent(slot.dataset.countFor) +
          "?_summary=count&_total=accurate",
        {
          headers: fhirHeaders(),
          credentials: "same-origin",
        }
      )
        .then(function (response) {
          return response.ok ? response.json() : null;
        })
        .then(function (bundle) {
          if (bundle && typeof bundle.total === "number") {
            slot.textContent = countFormat.format(bundle.total);
          }
        })
        .catch(function () {
          /* count stays blank */
        })
        .then(next);
    }
    for (var i = 0; i < CONCURRENCY; i++) next();
  }

  if (railList) {
    railList.addEventListener("click", function (event) {
      var item = event.target.closest("[data-rail-type]");
      if (!item || !urlInput) return;
      urlInput.value = "GET /" + item.dataset.railType;
      renderBuilder();
      runSearch("/" + encodeURIComponent(item.dataset.railType), false);
    });
  }
  if (railFilter && railList) {
    railFilter.addEventListener("input", function () {
      var needle = railFilter.value.trim().toLowerCase();
      railList.querySelectorAll("[data-rail-type]").forEach(function (item) {
        item.hidden =
          !!needle &&
          item.dataset.railType.toLowerCase().indexOf(needle) < 0;
      });
    });
  }

  /* ---- Results: the FHIR search response, rendered in-page ------------- */

  var results = {
    card: document.getElementById("query-results"),
    head: document.getElementById("query-results-head"),
    body: document.getElementById("query-results-body"),
    meta: document.getElementById("query-results-meta"),
    note: document.getElementById("query-results-note"),
    open: document.getElementById("query-results-open"),
    prev: document.getElementById("query-results-prev"),
    next: document.getElementById("query-results-next"),
  };

  /* Compact display heuristics for common FHIR shapes (HumanName,
   * CodeableConcept, Reference, Quantity); everything else is truncated
   * JSON rather than a blank cell. */
  function fmt(value) {
    if (value == null) return "";
    if (typeof value !== "object") return String(value);
    if (Array.isArray(value)) {
      if (!value.length) return "";
      var first = fmt(value[0]);
      return value.length > 1 ? first + " +" + (value.length - 1) : first;
    }
    if (value.family || value.given)
      return [value.family, (value.given || []).join(" ")]
        .filter(Boolean)
        .join(", ");
    if (value.text) return value.text;
    if (value.coding) return fmt(value.coding);
    if (value.display) return value.display;
    if (value.reference) return value.reference;
    if (value.value !== undefined && value.unit)
      return value.value + " " + value.unit;
    if (value.code) return value.code;
    var json = JSON.stringify(value);
    return json.length > 60 ? json.slice(0, 60) + "…" : json;
  }

  function elementColumns(query) {
    var columns = [];
    splitQuery(query).forEach(function (part) {
      if (part.key !== "_elements") return;
      part.value.split(",").forEach(function (el) {
        el = el.trim();
        if (el && el !== "id" && columns.indexOf(el) < 0) columns.push(el);
      });
    });
    return columns;
  }

  function cell(row, text, mono) {
    var td = document.createElement("td");
    if (mono) {
      var span = document.createElement("span");
      span.className = "url";
      span.textContent = text;
      td.appendChild(span);
    } else {
      td.textContent = text;
    }
    row.appendChild(td);
    return td;
  }

  function pagerLink(bundle, relation) {
    var links = (bundle && bundle.link) || [];
    for (var i = 0; i < links.length; i++) {
      if (links[i].relation === relation && links[i].url) return links[i].url;
    }
    return null;
  }

  function renderResults(path, ok, body) {
    var card = results.card;
    if (!card) return;
    card.hidden = false;
    results.open.href = path;
    results.head.textContent = "";
    results.body.textContent = "";
    results.meta.textContent = "";
    results.note.textContent = "";
    results.prev.hidden = true;
    results.next.hidden = true;

    if (!ok || !body || body.resourceType !== "Bundle") {
      var diagnostics =
        body &&
        body.issue &&
        body.issue[0] &&
        (body.issue[0].diagnostics ||
          (body.issue[0].details && body.issue[0].details.text));
      results.note.textContent = diagnostics || messages.msgError;
      return;
    }

    var parsed = parseSearchUrl(path) || { type: "", query: "" };
    var entries = body.entry || [];
    var primary = entries.filter(function (entry) {
      return (
        entry.resource && entry.resource.resourceType === parsed.type
      );
    });
    var included = entries.length - primary.length;

    var total =
      typeof body.total === "number" ? body.total : primary.length;
    var meta = card.dataset.msgTotal.replace("{count}", total);
    if (included > 0)
      meta += " · " + card.dataset.msgIncluded.replace("{count}", included);
    results.meta.textContent = meta;

    var columns = elementColumns(parsed.query);
    var headRow = document.createElement("tr");
    var th = document.createElement("th");
    th.textContent = "id";
    headRow.appendChild(th);
    columns.forEach(function (col) {
      var cellEl = document.createElement("th");
      cellEl.textContent = col;
      headRow.appendChild(cellEl);
    });
    var thUpdated = document.createElement("th");
    thUpdated.textContent = card.dataset.msgUpdated;
    headRow.appendChild(thUpdated);
    results.head.appendChild(headRow);

    primary.forEach(function (entry) {
      var resource = entry.resource;
      var row = document.createElement("tr");
      var idCell = document.createElement("td");
      var link = document.createElement("a");
      link.className = "url";
      link.href = "/" + parsed.type + "/" + resource.id;
      link.target = "_blank";
      link.rel = "noopener";
      link.textContent = resource.id || "";
      idCell.appendChild(link);
      row.appendChild(idCell);
      columns.forEach(function (col) {
        cell(row, fmt(resource[col]));
      });
      cell(
        row,
        resource.meta && resource.meta.lastUpdated
          ? whenText(resource.meta.lastUpdated)
          : ""
      );
      results.body.appendChild(row);
    });

    if (!primary.length) results.note.textContent = card.dataset.msgEmpty;

    var prevUrl = pagerLink(body, "previous");
    var nextUrl = pagerLink(body, "next");
    if (prevUrl) {
      results.prev.hidden = false;
      results.prev.dataset.url = prevUrl;
    }
    if (nextUrl) {
      results.next.hidden = false;
      results.next.dataset.url = nextUrl;
    }
  }

  /* Runs a search against the FHIR API and renders the Bundle in-page.
   * `record` adds it to the roaming recent list (explicit runs only, so
   * paging does not spam recents). */
  function runSearch(path, record) {
    if (!results.card) {
      window.open(path, "_blank", "noopener");
    } else {
      fetch(path, {
        headers: fhirHeaders(),
        credentials: "same-origin",
      })
        .then(function (response) {
          return response
            .json()
            .catch(function () {
              return null;
            })
            .then(function (body) {
              renderResults(path, response.ok, body);
            });
        })
        .catch(function () {
          renderResults(path, false, null);
        });
    }
    if (record) recordRecent(path);
  }

  results.card &&
    results.card.addEventListener("click", function (event) {
      var pager = event.target.closest("button[data-url]");
      if (pager) runSearch(pager.dataset.url, false);
    });

  /* ---- Recent searches & the saved list -------------------------------- */

  /* Last-accessed first; never-run entries follow, newest created first. */
  function compareEntries(a, b) {
    var aRun = a.entry.lastAccessedAt || "";
    var bRun = b.entry.lastAccessedAt || "";
    if (aRun !== bRun) return aRun > bRun ? -1 : 1;
    var aCreated = a.entry.createdAt || "";
    var bCreated = b.entry.createdAt || "";
    if (aCreated !== bCreated) return aCreated > bCreated ? -1 : 1;
    return a.id < b.id ? -1 : 1;
  }

  function whenText(iso) {
    var when = new Date(iso);
    if (isNaN(when.getTime())) return iso || "";
    var now = new Date();
    return when.toDateString() === now.toDateString()
      ? when.toLocaleTimeString(lang, { hour: "2-digit", minute: "2-digit" })
      : when.toLocaleDateString(lang);
  }

  function metaText(entry) {
    if (!entry.lastAccessedAt) return messages.msgNeverRun;
    var when = new Date(entry.lastAccessedAt);
    var text = isNaN(when.getTime())
      ? entry.lastAccessedAt
      : when.toLocaleString(lang);
    var runs = Number(entry.accessCount);
    return runs > 0 ? text + " · " + runs + "×" : text;
  }

  function button(label, action, resourceType, id) {
    var el = document.createElement("button");
    el.type = "button";
    el.className = "btn";
    el.textContent = label;
    el.dataset.action = action;
    el.dataset.type = resourceType;
    el.dataset.id = id;
    return el;
  }

  function renderRecent(doc) {
    if (!recentHost) return;
    recentHost.textContent = "";
    var list = recentSearches(doc);

    if (!list.length) {
      var empty = document.createElement("p");
      empty.className = "recent-empty";
      empty.textContent = recentHost.dataset.msgEmpty;
      recentHost.appendChild(empty);
      return;
    }

    list.forEach(function (item, index) {
      var row = document.createElement("div");
      row.className = "recent-item";

      var load = document.createElement("button");
      load.type = "button";
      load.className = "recent-item__query";
      load.dataset.recentLoad = String(index);
      load.textContent = "GET " + item.query;
      load.title = "GET " + item.query;

      var when = document.createElement("span");
      when.className = "recent-item__when";
      when.textContent = whenText(item.at);

      var del = document.createElement("button");
      del.type = "button";
      del.className = "recent-item__del";
      del.dataset.recentDelete = String(index);
      del.setAttribute("aria-label", recentHost.dataset.msgDelete);
      del.textContent = "×";

      row.appendChild(load);
      row.appendChild(when);
      row.appendChild(del);
      recentHost.appendChild(row);
    });
  }

  /* Prepends a run to recentSearches: dedupe by query, newest first, capped.
   * Reads fresh state first so runs from other tabs are not clobbered. */
  function recordRecent(path) {
    return fetchDocument().then(function (doc) {
      var list = recentSearches(doc).filter(function (item) {
        return item.query !== path;
      });
      list.unshift({ query: path, at: new Date().toISOString() });
      return patchDocument(
        { recentSearches: list.slice(0, MAX_RECENT) },
        0
      ).then(render);
    });
  }

  function render(doc) {
    clearError();
    renderRecent(doc);
    /* The Search page renders the builder and the recent list but no saved
     * list; there is nothing further to draw there. */
    if (!root) return;

    var byType = savedQueries(doc);
    root.textContent = "";

    var types = Object.keys(byType).sort();
    var total = 0;

    types.forEach(function (resourceType) {
      var entries = byType[resourceType];
      if (!entries || typeof entries !== "object" || Array.isArray(entries))
        return;
      var rows = Object.keys(entries)
        .map(function (id) {
          return { id: id, entry: entries[id] || {} };
        })
        .sort(compareEntries);
      if (!rows.length) return;
      total += rows.length;

      var group = document.createElement("section");
      group.className = "card query-group";
      var heading = document.createElement("h2");
      heading.className = "query-group__type";
      heading.textContent = resourceType;
      group.appendChild(heading);

      var list = document.createElement("ul");
      list.className = "query-list";
      rows.forEach(function (row) {
        var item = document.createElement("li");
        item.className = "query-row";

        var main = document.createElement("div");
        main.className = "query-row__main";
        var name = document.createElement("span");
        name.className = "query-row__name";
        name.textContent = row.entry.name || row.id;
        var query = document.createElement("code");
        query.className = "query-row__query";
        query.textContent = row.entry.query || "";
        main.appendChild(name);
        main.appendChild(query);

        var meta = document.createElement("span");
        meta.className = "query-row__meta";
        meta.textContent = metaText(row.entry);

        var actions = document.createElement("div");
        actions.className = "query-row__actions";
        actions.appendChild(
          button(messages.msgRun, "run", resourceType, row.id)
        );
        actions.appendChild(
          button(messages.msgRename, "rename", resourceType, row.id)
        );
        actions.appendChild(
          button(messages.msgDelete, "delete", resourceType, row.id)
        );

        item.appendChild(main);
        item.appendChild(meta);
        item.appendChild(actions);
        list.appendChild(item);
      });
      group.appendChild(list);
      root.appendChild(group);
    });

    if (!total) {
      var empty = document.createElement("p");
      empty.className = "query-empty";
      empty.textContent = messages.msgEmpty;
      root.appendChild(empty);
    }
  }

  /* Errors surface next to the query strip — the control they are almost
   * always about — on both pages that render the builder. */
  function showError(outcome, fallback) {
    if (!errorHost) return;
    errorHost.textContent =
      (outcome &&
        outcome.issue &&
        outcome.issue[0] &&
        outcome.issue[0].diagnostics) ||
      fallback ||
      messages.msgError;
    errorHost.hidden = false;
  }

  function clearError() {
    if (!errorHost) return;
    errorHost.textContent = "";
    errorHost.hidden = true;
  }

  function reload() {
    return fetchDocument()
      .then(render)
      .catch(function (failure) {
        var unavailable = failure && failure.unavailable;
        /* Saved queries need the per-user settings document; search does not.
         * Without the list, the page is the Search page — leave its form
         * alone and say nothing. */
        if (!root) return;
        root.textContent = "";
        var note = document.createElement("p");
        note.className = "query-empty";
        note.textContent = unavailable
          ? messages.msgUnavailable
          : messages.msgError;
        root.appendChild(note);
        if (unavailable && form) form.hidden = true;
      });
  }

  function mutate(resourceType, id, entry) {
    return patchEntry(resourceType, id, entry)
      .then(function (doc) {
        render(doc);
        return true;
      })
      .catch(function (failure) {
        return reload().then(function () {
          showError(failure && failure.outcome);
          return false;
        });
      });
  }

  function entryFor(doc, resourceType, id) {
    var entries = savedQueries(doc)[resourceType];
    return (entries && entries[id]) || null;
  }

  /* Loads a query into the builder without running it. */
  function loadIntoBuilder(path) {
    if (!urlInput) return;
    urlInput.value = "GET " + path;
    renderBuilder();
  }

  /* Copy the query exactly as shown: what gets copied is what would run. */
  var copyButton = document.getElementById("query-copy");
  if (copyButton && navigator.clipboard) {
    copyButton.addEventListener("click", function () {
      navigator.clipboard.writeText(urlInput.value || "");
    });
  } else if (copyButton) {
    copyButton.hidden = true;
  }

  /* Saved-list actions — only the Saved Queries page renders the list. */
  if (root) {
    root.addEventListener("click", function (event) {
      var target = event.target.closest("button[data-action]");
      if (!target) return;
      var resourceType = target.dataset.type;
      var id = target.dataset.id;

      fetchDocument().then(function (doc) {
        var entry = entryFor(doc, resourceType, id);
        if (!entry) {
          render(doc);
          return;
        }

        if (target.dataset.action === "run") {
          var path = searchPath(resourceType, entry.query || "");
          loadIntoBuilder(path);
          runSearch(path, true);
          mutate(resourceType, id, {
            lastAccessedAt: new Date().toISOString(),
            accessCount: (Number(entry.accessCount) || 0) + 1,
          });
        } else if (target.dataset.action === "rename") {
          var name = window.prompt(messages.msgRenamePrompt, entry.name || "");
          if (name === null) return;
          name = name.trim();
          if (!name || name === entry.name) return;
          mutate(resourceType, id, { name: name });
        } else if (target.dataset.action === "delete") {
          var label = (entry.name || id).toString();
          if (
            !window.confirm(messages.msgConfirmDelete.replace("{name}", label))
          )
            return;
          mutate(resourceType, id, null);
        }
      });
    });
  }

  if (recentHost) {
    recentHost.addEventListener("click", function (event) {
      var load = event.target.closest("[data-recent-load]");
      var del = event.target.closest("[data-recent-delete]");
      if (!load && !del) return;

      fetchDocument().then(function (doc) {
        var list = recentSearches(doc);
        if (load) {
          var item = list[Number(load.dataset.recentLoad)];
          if (item) {
            loadIntoBuilder(item.query);
            var dd = recentHost.closest("details");
            if (dd) dd.open = false;
            if (urlInput) urlInput.focus();
          }
          return;
        }
        list.splice(Number(del.dataset.recentDelete), 1);
        patchDocument({ recentSearches: list }, 0).then(render);
      });
    });
  }

  if (form) {
    form.addEventListener("submit", function (event) {
      event.preventDefault();
      var intent =
        (event.submitter && event.submitter.dataset.intent) || "run";
      var parsed = parseSearchUrl(form.elements.url.value);
      if (!parsed) {
        reload().then(function () {
          showError(null, messages.msgInvalidUrl);
        });
        return;
      }

      if (intent === "run") {
        runSearch(searchPath(parsed.type, parsed.query), true);
        return;
      }

      var name = form.elements.name.value.trim();
      if (!name) {
        form.elements.name.focus();
        return;
      }
      var id =
        Date.now().toString(36) +
        "-" +
        Math.random().toString(36).slice(2, 8);
      mutate(parsed.type, id, {
        name: name,
        query: parsed.query,
        createdAt: new Date().toISOString(),
      }).then(function (saved) {
        if (!saved) return;
        form.elements.name.value = "";
      });
    });
  }

  reload();
  window.setTimeout(hydrateCounts, 50);

  /* Deep link: /ui/queries?url=/Patient?name=smith loads the builder and
   * runs immediately — also what saved/recent entries could link to. */
  var deepLink = new URLSearchParams(window.location.search).get("url");
  if (deepLink && urlInput) {
    loadIntoBuilder(deepLink.replace(/^GET\s+/i, ""));
    var parsedDeep = parseSearchUrl(urlInput.value);
    if (parsedDeep)
      runSearch(searchPath(parsedDeep.type, parsedDeep.query), true);
  } else {
    renderBuilder();
  }
})();
