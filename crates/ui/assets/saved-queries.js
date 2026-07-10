/*
 * Saved FHIR queries (issue #234), backed by the per-user settings document.
 *
 * The page shell is server-rendered; this script owns the read/modify/write
 * cycle against /_user/settings. Unlike theme.js (a single last-write-wins
 * scalar), saved queries are structural state shared across tabs and devices,
 * so every write is a JSON merge patch scoped to one entry, conditional on the
 * document's ETag, and retried once against a fresh read when another writer
 * won the race (412).
 *
 * Document convention (see helios-persistence's user_settings module docs):
 * savedQueries.<ResourceType>.<id> = { name, query, createdAt,
 * lastAccessedAt?, accessCount? }. Entries are keyed by id precisely so a
 * merge patch can touch one of them without clobbering siblings.
 */
(function () {
  "use strict";

  var SETTINGS = "/_user/settings";
  var MAX_RETRIES = 2;

  var root = document.getElementById("saved-queries");
  var form = document.getElementById("saved-query-form");
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

  /* Merge-patches one entry (or null to delete it), conditional on the last
   * ETag we saw; on 412 re-reads and retries so an unrelated concurrent write
   * (another tab, the theme toggle) never surfaces to the user. */
  function patchEntry(resourceType, id, entry, attempt) {
    var patch = { savedQueries: {} };
    patch.savedQueries[resourceType] = {};
    patch.savedQueries[resourceType][id] = entry;

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
          return patchEntry(resourceType, id, entry, attempt + 1);
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

  function savedQueries(doc) {
    var byType = doc && doc.savedQueries;
    return byType && typeof byType === "object" && !Array.isArray(byType)
      ? byType
      : {};
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

  function metaText(entry) {
    if (!entry.lastAccessedAt) return messages.msgNeverRun;
    var when = new Date(entry.lastAccessedAt);
    var text = isNaN(when.getTime())
      ? entry.lastAccessedAt
      : when.toLocaleString(document.documentElement.lang || undefined);
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

  function render(doc) {
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

  function showError(outcome) {
    var text =
      (outcome &&
        outcome.issue &&
        outcome.issue[0] &&
        outcome.issue[0].diagnostics) ||
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
    return patchEntry(resourceType, id, entry, 0)
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
          "/" + encodeURIComponent(resourceType) + "?" + (entry.query || ""),
          "_blank",
          "noopener"
        );
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
        if (!window.confirm(messages.msgConfirmDelete.replace("{name}", label)))
          return;
        mutate(resourceType, id, null);
      }
    });
  });

  if (form) {
    form.addEventListener("submit", function (event) {
      event.preventDefault();
      var resourceType = form.elements.type.value.trim();
      var name = form.elements.name.value.trim();
      var query = form.elements.query.value.trim().replace(/^\?/, "");
      if (!resourceType || !name || !query) return;

      var id =
        Date.now().toString(36) +
        "-" +
        Math.random().toString(36).slice(2, 8);
      mutate(resourceType, id, {
        name: name,
        query: query,
        createdAt: new Date().toISOString(),
      }).then(function (saved) {
        if (!saved) return;
        form.reset();
        form.elements.type.focus();
      });
    });
  }

  reload();
})();
