/* Progressive enhancement for the Active SQL Exports job cards (#833 ticket
   03): "Copy job id" in each card's overflow menu. Every other action
   (Cancel/Retry/Run again/Remove from list) is a plain form and needs no
   script. This button itself is server-rendered `hidden` (and, on an
   in-progress card whose overflow would otherwise hold nothing else, its
   whole `<details class="menu">` starts `hidden` too) so a browser without
   the Clipboard API — or without JavaScript at all — never shows a control
   that cannot work. */
(function () {
  "use strict";

  function supportsClipboard() {
    return Boolean(window.navigator && navigator.clipboard && navigator.clipboard.writeText);
  }

  /* Reveals every Copy job id button under `root`, and the `<details>` menu
     wrapping it if that menu itself started hidden (the in-progress case,
     where Copy job id is the overflow's only item). */
  function reveal(root) {
    if (!supportsClipboard()) return;
    root.querySelectorAll("[data-copy-job-id]").forEach(function (button) {
      var details = button.closest("details.menu");
      if (details && details.hidden) details.hidden = false;
      button.hidden = false;
    });
  }

  function showCopied(button) {
    if (button.copyResetTimer) window.clearTimeout(button.copyResetTimer);
    var copiedLabel = button.dataset.copiedLabel;
    if (copiedLabel) button.textContent = copiedLabel;
    button.copyResetTimer = window.setTimeout(function () {
      var label = button.dataset.copyLabel;
      if (label) button.textContent = label;
      button.copyResetTimer = null;
    }, 2000);
  }

  document.addEventListener("click", function (event) {
    var button = event.target.closest("[data-copy-job-id]");
    if (!button || !supportsClipboard()) return;
    navigator.clipboard
      .writeText(button.dataset.copyJobId || "")
      .then(function () {
        showCopied(button);
      })
      .catch(function () {
        // Nothing to recover into: the button's label is left as-is, so a
        // denied clipboard permission is silent rather than misleadingly
        // claiming success.
      });
  });

  // A poll-refreshed card (in-progress, swapped every 5s) or a card whose
  // status just turned terminal arrives without this script re-running, so
  // its Copy button needs the same reveal pass htmx already gives every
  // other progressively-enhanced fragment in this crate.
  document.addEventListener("htmx:afterSwap", function (event) {
    var target = event.detail && event.detail.target;
    if (target) reveal(target);
  });

  reveal(document);
})();
