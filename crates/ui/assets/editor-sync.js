/* Guided form ↔ JSON cross-highlight for the resource editor: hovering or
   focusing a form row lights the JSON lines that hold that node, hovering a
   JSON line lights the row that edits it, and clicking a JSON line jumps to
   the row and focuses its input. Pure highlighting, delegated at the document
   so it works in the standalone editor and the Resources modal alike; with
   the script absent the editor is simply un-linked. */
(function () {
  "use strict";

  function grid(el) {
    return el && el.closest ? el.closest(".editor__grid") : null;
  }

  function clear(g) {
    g.querySelectorAll(".json-line--hit").forEach(function (n) {
      n.classList.remove("json-line--hit");
    });
    g.querySelectorAll(".editor-row--hit").forEach(function (n) {
      n.classList.remove("editor-row--hit");
    });
    g.__hitKey = null;
  }

  function within(path, ancestor) {
    return path === ancestor || path.indexOf(ancestor + ".") === 0;
  }

  /* Lights every JSON line inside the node at `path`, and keeps the first
     visible one in the JSON pane's viewport (the pane's own scroll only —
     never the page's). */
  function lightJson(g, path) {
    var first = null;
    g.querySelectorAll(".json-line[data-jpath]").forEach(function (line) {
      if (within(line.dataset.jpath, path)) {
        line.classList.add("json-line--hit");
        if (!first && !line.hidden) first = line;
      }
    });
    var view = g.querySelector(".json-view");
    if (first && view && view.scrollHeight > view.clientHeight) {
      var top = first.offsetTop - view.offsetTop;
      if (top < view.scrollTop || top > view.scrollTop + view.clientHeight - 24) {
        view.scrollTop = Math.max(0, top - view.clientHeight / 3);
      }
    }
  }

  /* The row for a JSON path: exact, else the nearest ancestor with a row
     (a scalar inside a CodeableConcept lands on the concept's row). */
  function rowForJsonPath(g, path) {
    while (path) {
      var rows = g.querySelectorAll(".editor-row[data-path]");
      for (var i = 0; i < rows.length; i++) {
        if (rows[i].dataset.path === path) return rows[i];
      }
      var cut = path.lastIndexOf(".");
      path = cut === -1 ? "" : path.slice(0, cut);
    }
    return null;
  }

  function handle(event) {
    var target = event.target;
    var row = target.closest ? target.closest(".editor-row[data-path]") : null;
    var line = target.closest ? target.closest(".json-line[data-jpath]") : null;
    var g = grid(row || line);
    if (!g) return;
    var key = row ? "r:" + row.dataset.path : "j:" + line.dataset.jpath;
    if (g.__hitKey === key) return;
    clear(g);
    g.__hitKey = key;
    if (row) {
      lightJson(g, row.dataset.path);
    } else {
      var match = rowForJsonPath(g, line.dataset.jpath);
      if (match) match.classList.add("editor-row--hit");
    }
  }

  document.addEventListener("mouseover", handle);
  document.addEventListener("focusin", handle);

  document.addEventListener("mouseout", function (event) {
    var from = event.target.closest
      ? event.target.closest(".editor-row[data-path], .json-line[data-jpath]")
      : null;
    var g = grid(from);
    if (!g) return;
    var to = event.relatedTarget;
    if (to && g.contains(to) && to.closest(".editor-row[data-path], .json-line[data-jpath]")) return;
    clear(g);
  });

  document.addEventListener("click", function (event) {
    if (event.target.closest && event.target.closest("[data-fold]")) return;
    var line = event.target.closest ? event.target.closest(".json-line[data-jpath]") : null;
    var g = grid(line);
    if (!g) return;
    var row = rowForJsonPath(g, line.dataset.jpath);
    if (!row) return;
    row.scrollIntoView({ block: "nearest" });
    row.classList.add("editor-row--hit");
    var input = row.querySelector("[data-set]");
    if (input) input.focus();
  });
})();
