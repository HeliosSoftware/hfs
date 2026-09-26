/*
 * Whole-row navigation for any table[data-row-navigation] (#610, generalized
 * for #1106).
 *
 * The row's first cell keeps its own real link for keyboard, assistive
 * technology, and no-JavaScript navigation. This handler only extends that
 * link's pointer target to the rest of the row — for every such table on the
 * page, including a tbody that is (re)rendered after load.
 */
(function () {
  "use strict";

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

  document.addEventListener("click", function (event) {
    var target = event.target;
    if (!target || !target.closest) return;
    var row = target.closest("table[data-row-navigation] > tbody > tr");
    if (!row) return;

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
