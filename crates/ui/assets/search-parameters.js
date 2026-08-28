/*
 * SearchParameter result-row navigation (#610).
 *
 * The first cell keeps the row's only real link for keyboard, assistive
 * technology, and no-JavaScript navigation. This handler only extends that
 * link's pointer target to the rest of the row.
 */
(function () {
  "use strict";

  var table = document.querySelector("table[data-row-navigation]");
  if (!table || !table.tBodies.length) return;
  var body = table.tBodies[0];
  var interactive = [
    "a",
    "button",
    "input",
    "select",
    "textarea",
    "label",
    "summary",
    '[role="button"]',
    '[role="link"]',
    '[contenteditable]:not([contenteditable="false"])',
  ].join(",");

  function rowContainsSelection(row) {
    var selection = window.getSelection && window.getSelection();
    if (!selection || selection.isCollapsed) return false;
    return Boolean(
      (selection.anchorNode && row.contains(selection.anchorNode)) ||
      (selection.focusNode && row.contains(selection.focusNode))
    );
  }

  body.addEventListener("click", function (event) {
    var target = event.target;
    if (!target || !target.closest) return;
    var row = target.closest("tr");
    if (!row || !body.contains(row)) return;

    // A drag may end over the real link. Cancel its native activation too.
    if (rowContainsSelection(row)) {
      event.preventDefault();
      return;
    }

    if (
      event.defaultPrevented ||
      event.button !== 0 ||
      event.ctrlKey ||
      event.metaKey ||
      event.shiftKey ||
      event.altKey ||
      target.closest(interactive)
    ) {
      return;
    }

    var link = row.querySelector("a.row-link[href]");
    if (link) link.click();
  });
})();
