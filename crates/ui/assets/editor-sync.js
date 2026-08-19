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
    g.querySelectorAll(".editor__source-mirror").forEach(function (m) {
      m.textContent = "";
    });
    g.__hitKey = null;
  }

  function within(path, ancestor) {
    return path === ancestor || path.indexOf(ancestor + ".") === 0;
  }

  /* Container-only reveal: brings `el` into `pane`'s viewport by moving the
     pane's own scroll — never the page's. */
  function reveal(pane, el) {
    if (!pane || !el || pane.scrollHeight <= pane.clientHeight) return;
    var delta = el.getBoundingClientRect().top - pane.getBoundingClientRect().top;
    var top = pane.scrollTop + delta;
    if (top < pane.scrollTop || top > pane.scrollTop + pane.clientHeight - 32) {
      pane.scrollTop = Math.max(0, top - pane.clientHeight / 3);
    }
  }

  /* Lights every JSON line inside the node at `path`, and keeps the first
     visible one in the JSON pane's viewport. */
  function lightJson(g, path) {
    var first = null;
    g.querySelectorAll(".json-line[data-jpath]").forEach(function (line) {
      if (within(line.dataset.jpath, path)) {
        line.classList.add("json-line--hit");
        if (!first && !line.hidden) first = line;
      }
    });
    reveal(g.querySelector(".json-view"), first);
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
      if (!lightRaw(g, row.dataset.path)) lightJson(g, row.dataset.path);
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

  /* ---- raw-mode highlight ----------------------------------------------- */

  /* The character range a dotted path occupies in the JSON text: from its key
     (or its opening bracket, for array items) to the end of its value. Same
     structure-only scan as pathAtCaret, watching for the moment the walk
     enters and leaves the target's subtree. */
  function rangeOfPath(text, target) {
    var frames = [];
    var lastStr = null;
    var lastStrStart = -1;
    var start = -1;
    function cur() {
      if (!frames.length) return "";
      var parts = [];
      for (var j = 1; j < frames.length; j++) {
        if (frames[j].segment != null) parts.push(frames[j].segment);
      }
      var tip = frames[frames.length - 1];
      var c = tip.kind === "o" ? tip.key : String(tip.index);
      if (c != null) parts.push(c);
      return parts.join(".");
    }
    function inside() {
      var p = cur();
      return !!p && within(p, target);
    }
    for (var i = 0; i < text.length; i++) {
      var ch = text[i];
      if (ch === '"') {
        var sStart = i;
        var end = i + 1;
        var esc = false;
        var buf = "";
        while (end < text.length) {
          var c2 = text[end];
          if (esc) { buf += c2; esc = false; }
          else if (c2 === "\\") esc = true;
          else if (c2 === '"') break;
          else buf += c2;
          end++;
        }
        lastStr = buf;
        lastStrStart = sStart;
        i = end;
        continue;
      }
      if ("{}[]:,".indexOf(ch) === -1) continue;
      var top = frames[frames.length - 1];
      var before = inside();
      var closedOwn = false;
      if (ch === ":") {
        if (top && top.kind === "o") top.key = lastStr;
      } else if (ch === ",") {
        if (top && top.kind === "a") top.index++;
        else if (top) top.key = null;
      } else if (ch === "{" || ch === "[") {
        var seg = top ? (top.kind === "o" ? top.key : String(top.index)) : null;
        frames.push({ kind: ch === "{" ? "o" : "a", key: null, index: 0, segment: seg });
      } else {
        // The bracket belongs to the target only when it closes the target's
        // own container — closing an ancestor while a scalar tail is inside
        // the subtree must not drag the ancestor's bracket into the range.
        var segments = [];
        for (var s = 1; s < frames.length; s++) {
          if (frames[s].segment != null) segments.push(frames[s].segment);
        }
        closedOwn = segments.join(".") === target;
        frames.pop();
      }
      var after = inside();
      if (!before && after && start === -1) {
        if (ch === ":" && lastStrStart !== -1) start = lastStrStart;
        else if (ch === ",") {
          start = i + 1;
          while (start < text.length && /\s/.test(text[start])) start++;
        } else start = i;
      }
      if (before && !after && start !== -1) {
        return { start: start, end: ch === "," || (ch !== ":" && !closedOwn && "}]".indexOf(ch) !== -1) ? i : i + 1 };
      }
    }
    return start === -1 ? null : { start: start, end: text.length };
  }

  /* A mirror <pre> behind the textarea carries the same text, invisible except
     the <mark> around the highlighted range. Created on first use, cleared on
     every keystroke (stale offsets would mark the wrong characters). */
  function mirrorFor(rawPane, source) {
    var mirror = rawPane.querySelector(".editor__source-mirror");
    if (!mirror) {
      mirror = document.createElement("pre");
      mirror.className = "editor__source-mirror";
      mirror.setAttribute("aria-hidden", "true");
      rawPane.insertBefore(mirror, source);
      rawPane.classList.add("editor-json__raw--mirrored");
      source.addEventListener("scroll", function () {
        mirror.scrollTop = source.scrollTop;
        mirror.scrollLeft = source.scrollLeft;
      });
      source.addEventListener("input", function () {
        mirror.textContent = "";
      });
    }
    mirror.style.height = source.offsetHeight + "px";
    return mirror;
  }

  /* Highlights `path` inside the raw textarea. Returns false when raw mode is
     closed, so the caller falls back to the fold-view lines. */
  function lightRaw(g, path) {
    var rawPane = g.querySelector("#editor-json-raw");
    if (!rawPane || rawPane.hidden) return false;
    var source = rawPane.querySelector(".editor__source");
    if (!source) return false;
    var mirror = mirrorFor(rawPane, source);
    var range = rangeOfPath(source.value, path);
    mirror.textContent = "";
    if (!range) return true;
    mirror.appendChild(document.createTextNode(source.value.slice(0, range.start)));
    var mark = document.createElement("mark");
    mark.textContent = source.value.slice(range.start, range.end);
    mirror.appendChild(mark);
    mirror.appendChild(document.createTextNode(source.value.slice(range.end)));
    if (mark.offsetTop < source.scrollTop || mark.offsetTop > source.scrollTop + source.clientHeight - 40) {
      source.scrollTop = Math.max(0, mark.offsetTop - source.clientHeight / 3);
    }
    mirror.scrollTop = source.scrollTop;
    mirror.scrollLeft = source.scrollLeft;
    return true;
  }

  /* ---- raw-edit caret follow ------------------------------------------- */

  /* While "Edit raw" is open the JSON pane is one textarea, so there are no
     lines to light — instead the caret drives the form: a structure-only scan
     of the text up to the caret yields the dotted path of the node it sits
     in, and that row lights up. The scanner tracks strings and escapes, so a
     half-typed (invalid) document still resolves to the enclosing node. */
  function pathAtCaret(text, caret) {
    var frames = [];
    var inStr = false;
    var esc = false;
    var lastStr = null;
    var limit = Math.min(caret, text.length);
    for (var i = 0; i < limit; i++) {
      var ch = text[i];
      if (inStr) {
        if (esc) esc = false;
        else if (ch === "\\") esc = true;
        else if (ch === '"') inStr = false;
        continue;
      }
      if (ch === '"') {
        // Read the whole string ahead so a key the caret sits inside still counts.
        var end = i + 1;
        var buf = "";
        var esc2 = false;
        while (end < text.length) {
          var c2 = text[end];
          if (esc2) { buf += c2; esc2 = false; }
          else if (c2 === "\\") esc2 = true;
          else if (c2 === '"') break;
          else buf += c2;
          end++;
        }
        lastStr = buf;
        if (end >= limit) {
          // Caret inside this string. If it is a key (a colon follows), bind
          // it so the caret lands on that entry's row, not the container's.
          var after = end + 1;
          while (after < text.length && /\s/.test(text[after])) after++;
          var owner = frames[frames.length - 1];
          if (text[after] === ":" && owner && owner.kind === "o") owner.key = lastStr;
          break;
        }
        i = end;
        continue;
      }
      var top = frames[frames.length - 1];
      if (ch === ":") {
        if (top && top.kind === "o") top.key = lastStr;
      } else if (ch === ",") {
        if (top && top.kind === "a") top.index++;
        else if (top) top.key = null;
      } else if (ch === "{" || ch === "[") {
        var segment = top ? (top.kind === "o" ? top.key : String(top.index)) : null;
        frames.push({ kind: ch === "{" ? "o" : "a", key: null, index: 0, segment: segment });
      } else if (ch === "}" || ch === "]") {
        frames.pop();
      }
    }
    if (!frames.length) return "";
    var parts = [];
    for (var j = 1; j < frames.length; j++) {
      if (frames[j].segment != null) parts.push(frames[j].segment);
    }
    var tip = frames[frames.length - 1];
    var current = tip.kind === "o" ? tip.key : String(tip.index);
    if (current != null) parts.push(current);
    return parts.join(".");
  }

  /* ---- live raw → form sync -------------------------------------------- */

  /* As soon as the textarea parses as JSON, the guided form catches up: the
     document is re-rendered server-side and only the form pane (plus the
     hidden document field and the binding datalists) is swapped, so the
     textarea — and the caret in it — is never touched. Invalid JSON simply
     waits; the form always reflects the last parseable state. */
  var syncTimer = null;
  var syncSeq = 0;
  var lastSynced = null;
  document.addEventListener("input", function (event) {
    var source = event.target;
    if (!source.classList || !source.classList.contains("editor__source")) return;
    clearTimeout(syncTimer);
    syncTimer = setTimeout(function () {
      var text = source.value;
      var parsed;
      try {
        parsed = JSON.parse(text);
      } catch (invalid) {
        return;
      }
      var canonical = JSON.stringify(parsed);
      if (canonical === lastSynced) return;
      var g = grid(source);
      if (!g || !g.parentElement) return;
      var seq = ++syncSeq;
      var form = new URLSearchParams();
      form.set("doc", text);
      form.set("op", "");
      fetch("/ui/editor/render", { method: "POST", body: form })
        .then(function (r) { return r.text(); })
        .then(function (html) {
          if (seq !== syncSeq || !document.contains(source)) return;
          lastSynced = canonical;
          var fresh = new DOMParser().parseFromString(html, "text/html");
          var container = g.parentElement;
          var freshDoc = fresh.querySelector("#editor-form");
          var oldDoc = container.querySelector("#editor-form");
          if (freshDoc && oldDoc) oldDoc.replaceWith(freshDoc);
          var freshForm = fresh.querySelector("section.editor-form");
          var oldForm = g.querySelector("section.editor-form");
          if (freshForm && oldForm) oldForm.replaceWith(freshForm);
          // Required-binding datalists ride at the fragment's top level.
          container.querySelectorAll(":scope > datalist").forEach(function (d) { d.remove(); });
          fresh.querySelectorAll("body > datalist").forEach(function (d) { container.appendChild(d); });
          g.__hitKey = null;
          var path = pathAtCaret(text, source.selectionStart || 0);
          var row = path ? rowForJsonPath(g, path) : null;
          if (row) {
            row.classList.add("editor-row--hit");
            reveal(g.querySelector(".editor-tree"), row);
          }
        })
        .catch(function () {});
    }, 600);
  });

  var caretTimer = null;
  document.addEventListener("selectionchange", function () {
    var source = document.activeElement;
    if (!source || !source.classList || !source.classList.contains("editor__source")) return;
    clearTimeout(caretTimer);
    caretTimer = setTimeout(function () {
      var g = grid(source);
      if (!g) return;
      var path = pathAtCaret(source.value, source.selectionStart || 0);
      var key = "c:" + path;
      if (g.__hitKey === key) return;
      clear(g);
      if (!path) return;
      g.__hitKey = key;
      var row = rowForJsonPath(g, path);
      if (row) {
        row.classList.add("editor-row--hit");
        reveal(g.querySelector(".editor-tree"), row);
      }
    }, 120);
  });
})();
