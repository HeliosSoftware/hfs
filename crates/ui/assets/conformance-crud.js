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
    if (!btn || !window.fetch) return;
    if (!window.confirm(btn.dataset.confirm)) return;

    var headers = { Accept: "application/fhir+json" };
    if (TENANT) headers["X-Tenant-ID"] = TENANT;
    btn.disabled = true;
    fetch("/" + btn.dataset.type + "/" + btn.dataset.id, { method: "DELETE", headers: headers })
      .then(function (response) {
        if (!response.ok) throw new Error("HTTP " + response.status);
        window.location = btn.dataset.redirect;
      })
      .catch(function (error) {
        btn.disabled = false;
        window.alert(btn.dataset.failed + " (" + error.message + ")");
      });
  });
})();
