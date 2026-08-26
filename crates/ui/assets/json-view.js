/* Shared, delegated behavior for every server-rendered JSON view (#678). */
(function () {
  "use strict";

  function opener(view, foldId) {
    var candidates = view.querySelectorAll(".json-line[data-fold-id]");
    for (var i = 0; i < candidates.length; i++) {
      if (candidates[i].dataset.foldId === foldId) return candidates[i];
    }
    return null;
  }

  function reflow(view) {
    var openers = Object.create(null);
    view.querySelectorAll(".json-line[data-fold-id]").forEach(function (line) {
      if (line.dataset.foldId) openers[line.dataset.foldId] = line;
    });
    view.querySelectorAll(".json-line").forEach(function (line) {
      var parents = (line.dataset.parents || "").split(" ");
      line.hidden = parents.some(function (parentId) {
        var parent = parentId && openers[parentId];
        return !!(parent && parent.classList.contains("json-line--collapsed"));
      });
    });
  }

  function setCollapsed(view, line, collapse) {
    line.classList.toggle("json-line--collapsed", collapse);
    var arrow = line.querySelector("[data-fold]");
    if (arrow) arrow.setAttribute("aria-expanded", collapse ? "false" : "true");
    reflow(view);
  }

  document.addEventListener("click", function (event) {
    var arrow = event.target.closest && event.target.closest("[data-fold]");
    if (arrow) {
      var view = arrow.closest(".json-view");
      if (!view) return;
      var line = opener(view, arrow.dataset.fold);
      if (!line) return;
      setCollapsed(view, line, !line.classList.contains("json-line--collapsed"));
      return;
    }

    var control = event.target.closest && event.target.closest("[data-json-fold]");
    if (!control) return;
    var scope = control.closest("[data-json-view-scope]");
    var target = scope && scope.querySelector(".json-view");
    if (!target) return;
    var collapse = control.dataset.jsonFold === "all";
    target.querySelectorAll(".json-line--foldable").forEach(function (line) {
      // Keep the root visible when applying Collapse all.
      if (!line.dataset.parents) return;
      line.classList.toggle("json-line--collapsed", collapse);
      var lineArrow = line.querySelector("[data-fold]");
      if (lineArrow) lineArrow.setAttribute("aria-expanded", collapse ? "false" : "true");
    });
    reflow(target);
  });
})();
