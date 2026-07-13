/*
 * Saved FHIR queries (issue #234), backed by the per-user settings document.
 *
 * The page shell is server-rendered; this script owns the read/modify/write
 * cycle against /_user/settings. Unlike theme.js (a single last-write-wins
 * scalar), saved queries are structural state shared across tabs and devices,
 * so every write is a JSON merge patch conditional on the document's ETag,
 * retried once against a fresh read when another writer won the race (412).
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
  var MAX_RETRIES = 2;
  var MAX_RECENT = 10;

  var root = document.getElementById("saved-queries");
  var form = document.getElementById("saved-query-form");
  var recentHost = document.getElementById("recent-searches");
  if (!root || !window.fetch) return;

  var messages = root.dataset;
  var etag = null;

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

  var lang = document.documentElement.lang || undefined;

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
    var byType = savedQueries(doc);
    root.textContent = "";
    renderRecent(doc);

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

  function showError(outcome, fallback) {
    var text =
      (outcome &&
        outcome.issue &&
        outcome.issue[0] &&
        outcome.issue[0].diagnostics) ||
      fallback ||
      messages.msgError;
    var note = document.createElement("p");
    note.className = "query-error";
    note.setAttribute("role", "alert");
    note.textContent = text;
    root.insertBefore(note, root.firstChild);
  }

  function reload() {
    return fetchDocument()
      .then(render)
      .catch(function (failure) {
        root.textContent = "";
        var note = document.createElement("p");
        note.className = "query-empty";
        note.textContent =
          failure && failure.unavailable
            ? messages.msgUnavailable
            : messages.msgError;
        root.appendChild(note);
        if (failure && failure.unavailable && form) form.hidden = true;
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
        window.open(
          searchPath(resourceType, entry.query || ""),
          "_blank",
          "noopener"
        );
        mutate(resourceType, id, {
          lastAccessedAt: new Date().toISOString(),
          accessCount: (Number(entry.accessCount) || 0) + 1,
        }).then(function () {
          return recordRecent(searchPath(resourceType, entry.query || ""));
        });
      } else if (target.dataset.action === "rename") {
        var name = window.prompt(messages.msgRenamePrompt, entry.name || "");
        if (name === null) return;
        name = name.trim();
        if (!name || name === entry.name) return;
        mutate(resourceType, id, { name: name });
      } else if (target.dataset.action === "delete") {
        var label = (entry.name || id).toString();
        if (!window.confirm(messages.msgConfirmDelete.replace("{name}", label)))
          return;
        mutate(resourceType, id, null);
      }
    });
  });

  if (recentHost) {
    recentHost.addEventListener("click", function (event) {
      var load = event.target.closest("[data-recent-load]");
      var del = event.target.closest("[data-recent-delete]");
      if (!load && !del) return;

      fetchDocument().then(function (doc) {
        var list = recentSearches(doc);
        if (load) {
          var item = list[Number(load.dataset.recentLoad)];
          if (item && form) {
            form.elements.url.value = "GET " + item.query;
            var dd = recentHost.closest("details");
            if (dd) dd.open = false;
            form.elements.url.focus();
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
        window.open(
          searchPath(parsed.type, parsed.query),
          "_blank",
          "noopener"
        );
        recordRecent(searchPath(parsed.type, parsed.query));
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
})();
