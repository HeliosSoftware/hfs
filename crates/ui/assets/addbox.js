/* Close behavior for <details class="addbox"> disclosures (#545) and
   <details class="menu"> dropdowns (tenant and version pickers, recent
   searches): Esc closes the open panel, a [data-addbox-close] control closes
   its own panel, and a click outside any open panel closes it. The disclosures
   stay fully usable without this script — it only adds ways out. */
(function () {
  "use strict";

  var OPEN = "details.addbox[open], details.menu[open]";

  function close(box) {
    box.removeAttribute("open");
    /* Every close this script performs is a dismissal, so the dialog starts
       blank next time (#682). The failure path never comes through here — an
       errored submit re-renders inside the still-open panel — and success
       paths (e.g. tenants.js) reset on their own before closing. */
    box.querySelectorAll("form").forEach(function (form) {
      form.reset();
    });
    /* Keep focus in a sensible place after Esc / the × removes the panel the
       focus was in. An outside click keeps its own target's focus. */
    var summary = box.querySelector("summary");
    if (summary && box.contains(document.activeElement)) summary.focus();
  }

  document.addEventListener("keydown", function (event) {
    if (event.key !== "Escape") return;
    document.querySelectorAll(OPEN).forEach(close);
  });

  document.addEventListener("click", function (event) {
    var closer = event.target.closest("[data-addbox-close]");
    if (closer) {
      var own = closer.closest("details.addbox, details.menu");
      if (own) {
        event.preventDefault();
        close(own);
      }
      return;
    }
    document.querySelectorAll(OPEN).forEach(function (box) {
      if (!box.contains(event.target)) close(box);
    });
  });
})();
