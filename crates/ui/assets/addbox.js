/* Close behavior for <details class="addbox"> disclosures (#545): Esc closes
   the open panel, a [data-addbox-close] control closes its own panel, and a
   click outside any open panel closes it. The disclosures stay fully usable
   without this script — it only adds ways out. */
(function () {
  "use strict";

  function close(box) {
    box.removeAttribute("open");
  }

  document.addEventListener("keydown", function (event) {
    if (event.key !== "Escape") return;
    document.querySelectorAll("details.addbox[open]").forEach(close);
  });

  document.addEventListener("click", function (event) {
    var closer = event.target.closest("[data-addbox-close]");
    if (closer) {
      var own = closer.closest("details.addbox");
      if (own) {
        event.preventDefault();
        close(own);
      }
      return;
    }
    document.querySelectorAll("details.addbox[open]").forEach(function (box) {
      if (!box.contains(event.target)) close(box);
    });
  });
})();
