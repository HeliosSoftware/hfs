/*
 * New / Edit / Delete for the conformance viewers (#237, #238). Create and
 * edit deep-link into the schema-driven editor page; delete goes straight to
 * the ordinary FHIR API, then reloads the page with `refresh=1` so the server
 * drops its cached snapshot and re-fetches.
 */
(function () {
  "use strict";

  /* The effective tenant, stamped by the server (#344); FHIR calls carry it. */
  var TENANT = (document.querySelector('meta[name="hfs-tenant"]') || {}).content || "";

  document.addEventListener("click", function (event) {
    var btn = event.target.closest ? event.target.closest("[data-crud-delete]") : null;
    if (!btn || !window.fetch || !window.hfsBusy) return;
    if (!window.confirm(btn.dataset.confirm)) return;

    var headers = { Accept: "application/fhir+json" };
    if (TENANT) headers["X-Tenant-ID"] = TENANT;
    /* The shared busy state (#679); the guard is per-button, so unrelated
       rows stay independently deletable. */
    window.hfsBusy.during([btn], function () {
      return fetch("/" + btn.dataset.type + "/" + btn.dataset.id, { method: "DELETE", headers: headers })
        .then(function (response) {
          if (!response.ok) throw new Error("HTTP " + response.status);
          window.location = btn.dataset.redirect;
          /* Navigating away: never settle, so the button stays inert until
             the page unloads instead of re-arming mid-navigation. */
          return new Promise(function () {});
        })
        .catch(function (error) {
          /* Settling here is what re-enables the button via the helper. The
             shared error treatment goes next to the button that failed
             (#676) — not a native alert dialog. Replaced on the next
             attempt. */
          var existing = btn.parentNode.querySelector(".alert");
          if (existing) existing.remove();
          var note = document.createElement("span");
          note.className = "alert alert--inline";
          note.setAttribute("role", "alert");
          note.textContent = btn.dataset.failed + " (" + error.message + ")";
          btn.insertAdjacentElement("afterend", note);
        });
    });
  });
})();
