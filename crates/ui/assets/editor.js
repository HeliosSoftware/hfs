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

  var resourceType = messages.type;
  var resourceId = messages.id;

  function say(text, kind) {
    status.textContent = text || "";
    status.className = "editor__status" + (kind ? " editor__status--" + kind : "");
  }

  /* ---- the round trip -------------------------------------------------- */

  /* Posts the document plus one mutation, and swaps in the re-rendered body.
   * `op` is empty for a plain re-render (first load, or after a source edit). */
  function send(op, fields) {
    /* URLSearchParams, not FormData: fetch sends FormData as multipart, and the
     * server takes an urlencoded form. */
    var form = new URLSearchParams();
    form.set("doc", currentDocument());
    form.set("op", op || "");
    Object.keys(fields || {}).forEach(function (key) {
      form.set(key, fields[key]);
    });

    return fetch("/ui/editor/render", { method: "POST", body: form })
      .then(function (response) {
        return response.text();
      })
      .then(function (html) {
        body.innerHTML = html;
        applyView();
      });
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

  /* ---- loading --------------------------------------------------------- */

  function load() {
    if (!resourceId) {
      // A new, empty resource of the requested type. Seed the hidden field the
      // fragment will replace, then render it.
      var seed = new URLSearchParams();
      seed.set("doc", JSON.stringify({ resourceType: resourceType }));
      seed.set("op", "");
      return fetch("/ui/editor/render", { method: "POST", body: seed })
        .then(function (r) { return r.text(); })
        .then(function (html) { body.innerHTML = html; applyView(); });
    }
    return fetch("/" + resourceType + "/" + resourceId, {
      headers: fhirHeaders(),
    })
      .then(function (response) {
        if (!response.ok) throw new Error(String(response.status));
        return response.json();
      })
      .then(function (resource) {
        subject.textContent =
          resourceType +
          "/" +
          (resource.id || "") +
          (resource.meta && resource.meta.lastUpdated
            ? " · " + new Date(resource.meta.lastUpdated).toLocaleString()
            : "");
        loadVersions();
        return renderDocument(resource);
      })
      .catch(function () {
        say(messages.msgLoadError, "error");
      });
  }

  /* Renders a document into the editor body (JSON + form). */
  function renderDocument(resource) {
    var form = new URLSearchParams();
    form.set("doc", JSON.stringify(resource));
    form.set("op", "");
    return fetch("/ui/editor/render", { method: "POST", body: form })
      .then(function (r) { return r.text(); })
      .then(function (html) { body.innerHTML = html; applyView(); });
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
        renderVersions((bundle && bundle.entry) || []);
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
        renderDocument(resource);
        subject.textContent = resourceType + "/" + resourceId + " · v" + version;
      });
      versionsHost.appendChild(row);
    });
  }

  /* ---- JSON view: folding + raw editing -------------------------------- */

  function applyView() {
    /* No-op retained as the render hook the round trip calls after a swap. */
  }

  /* Collapses or expands a container: hides every line inside it and shows the
   * `{ … }` / `[ N ]` summary on the opening line. Pure CSS class toggling —
   * the fold state is a data attribute JS reads. */
  function toggleFold(foldId, collapse) {
    var opener = root.querySelector('.json-line[data-fold-id="' + foldId + '"]');
    if (opener) opener.classList.toggle("json-line--collapsed", collapse);
    root.querySelectorAll(".json-line").forEach(function (line) {
      var parents = (line.dataset.parents || "").split(" ");
      if (parents.indexOf(foldId) !== -1) {
        // A line stays hidden if ANY ancestor is collapsed.
        line.hidden = anyAncestorCollapsed(line);
      }
    });
  }

  function anyAncestorCollapsed(line) {
    var parents = (line.dataset.parents || "").split(" ");
    for (var i = 0; i < parents.length; i++) {
      if (!parents[i]) continue;
      var opener = root.querySelector('.json-line[data-fold-id="' + parents[i] + '"]');
      if (opener && opener.classList.contains("json-line--collapsed")) return true;
    }
    return false;
  }

  function foldAll(collapse) {
    root.querySelectorAll(".json-line--foldable").forEach(function (opener) {
      // Never fold the root object away entirely.
      if (!opener.dataset.parents) return;
      opener.classList.toggle("json-line--collapsed", collapse);
    });
    root.querySelectorAll(".json-line").forEach(function (line) {
      if (line.dataset.parents) line.hidden = anyAncestorCollapsed(line);
    });
  }

  root.addEventListener("click", function (event) {
    /* Fold arrow. */
    var arrow = event.target.closest("[data-fold]");
    if (arrow) {
      var id = arrow.dataset.fold;
      var opener = root.querySelector('.json-line[data-fold-id="' + id + '"]');
      toggleFold(id, opener && !opener.classList.contains("json-line--collapsed"));
      return;
    }
    var foldAllBtn = event.target.closest("[data-json-fold]");
    if (foldAllBtn) {
      foldAll(foldAllBtn.dataset.jsonFold === "all");
      return;
    }
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
        var source = document.getElementById("editor-source");
        var field = document.getElementById("editor-doc");
        if (source && field) field.value = source.value;
        send("");
      }
      return;
    }

    /* ---- structural mutations: each is one round trip ------------------ */

    var add = event.target.closest("[data-add]");
    if (add) {
      send("add", { path: add.dataset.add, name: add.dataset.name, slice: add.dataset.slice || "" });
      return;
    }

    var remove = event.target.closest("[data-remove]");
    if (remove) {
      send("remove", { path: remove.dataset.remove });
      return;
    }

    var extension = event.target.closest("[data-extension]");
    if (extension) {
      var panel = extension.closest(".editor-add__ext");
      var url = panel ? panel.querySelector(".editor-add__ext-url").value.trim() : "";
      send("extension", { path: extension.dataset.extension, url: url });
      return;
    }

    if (event.target.id === "editor-save") save();
    if (event.target.id === "editor-delete") remove_resource();
  });

  /* A value[x]: the user picks the type, the server creates the branch. */
  root.addEventListener("change", function (event) {
    var choose = event.target.closest("[data-choose]");
    if (choose && choose.value) {
      send("choose", {
        path: choose.dataset.choose,
        name: choose.dataset.declarer,
        arm: choose.value,
      });
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

  /* Typeahead over the "add" list -- the only thing here that is purely
   * cosmetic, and the only thing that would be silly to round-trip. */
  root.addEventListener("input", function (event) {
    var filter = event.target.closest(".editor-add__filter");
    if (!filter) return;
    var needle = filter.value.trim().toLowerCase();
    var panel = filter.closest(".editor-add__panel");
    panel.querySelectorAll("[data-add-name]").forEach(function (item) {
      item.hidden = needle && item.dataset.addName.toLowerCase().indexOf(needle) === -1;
    });
  });

  /* ---- saving ---------------------------------------------------------- */

  function save() {
    var doc = currentDocument();
    var parsed;
    try {
      parsed = JSON.parse(doc);
    } catch (error) {
      say(messages.msgSaveInvalid || String(error), "error");
      return;
    }

    // Validate the exact document being saved by re-rendering it (this also
    // commits a raw edit), then refuse to persist a new version while the
    // editor reports any issue — an invalid resource must not reach history.
    send("").then(function () {
      var form = document.getElementById("editor-form");
      var errors = form ? parseInt(form.dataset.errorCount || "0", 10) : 0;
      if (errors > 0) {
        say(messages.msgSaveBlocked, "error");
        return;
      }

      var isNew = !parsed.id;
      var url = "/" + resourceType + (isNew ? "" : "/" + parsed.id);

      fetch(url, {
        method: isNew ? "POST" : "PUT",
        headers: fhirHeaders({ "Content-Type": "application/fhir+json" }),
        body: doc,
      })
        .then(function (response) {
          return response.json().then(function (payload) {
            return { ok: response.ok, payload: payload };
          });
        })
        .then(function (result) {
          if (!result.ok) {
            // The server refused it. Show its OperationOutcome, not our guess.
            var issue = result.payload && result.payload.issue && result.payload.issue[0];
            say(
              (issue && (issue.diagnostics || (issue.details && issue.details.text))) ||
                "",
              "error"
            );
            return;
          }
          say(messages.msgSaved, "ok");
          if (result.payload && result.payload.id && !parsed.id) {
            resourceId = result.payload.id;
          }
        })
        .catch(function (error) {
          say(String(error), "error");
        });
    });
  }

  function remove_resource() {
    var parsed = JSON.parse(currentDocument() || "{}");
    if (!parsed.id) return;
    if (!window.confirm(messages.msgConfirmDelete)) return;

    fetch("/" + resourceType + "/" + parsed.id, { method: "DELETE", headers: fhirHeaders() })
      .then(function (response) {
        if (response.ok) window.location.href = "/ui/queries";
      })
      .catch(function (error) {
        say(String(error), "error");
      });
  }

  load();
})();
