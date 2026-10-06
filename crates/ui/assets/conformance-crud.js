/*
 * New / Edit / Delete for the conformance viewers (#237, #238). Create and
 * edit deep-link into the schema-driven editor page; delete goes straight to
 * the ordinary FHIR API, then reloads the page with `refresh=1` so the server
 * drops its cached snapshot and re-fetches.
 */
(function () {
  "use strict";

  /* Boosted navigation executes this body script again. The document keeps
     its delegated listener across swaps; a property is not copied into
     htmx's HTML history snapshots like a data attribute would be. */
  if (document.hfsConformanceCrudInstalled) return;
  document.hfsConformanceCrudInstalled = true;

  /* The effective tenant, stamped by the server (#344); FHIR calls carry it. */
  var TENANT = (document.querySelector('meta[name="hfs-tenant"]') || {}).content || "";

  // The server supplies an origin for no-JS links. Capture the browser's
  // current query/hash too, before htmx handles a boosted Edit link.
  document.addEventListener("click", function (event) {
    var link = event.target.closest ? event.target.closest("a[data-editor-link]") : null;
    if (!link) return;
    var target = new URL(link.href, window.location.origin);
    var current = new URL(window.location.href);
    current.searchParams.delete("return_to");
    current.searchParams.delete("saved");
    // A section root may restore rail.last without adding the selected id
    // to the browser URL. Keep the server-resolved selection as this origin.
    var own = document.querySelector('input[name="current_path"]');
    if (own) {
      var selected = new URL(own.value, window.location.origin);
      if (selected.pathname === current.pathname) {
        ["vd", "lib"].forEach(function (key) {
          if (selected.searchParams.has(key)) current.searchParams.set(key, selected.searchParams.get(key));
        });
      }
    }
    target.searchParams.set("return_to", current.pathname + current.search + current.hash);
    link.href = target.pathname + target.search + target.hash;
  }, true);

  document.addEventListener("click", function (event) {
    var btn = event.target.closest ? event.target.closest("[data-crud-delete]") : null;
    if (!btn || !window.fetch || !window.hfsBusy) return;
    /* The shared in-page confirmation (#1667), not the browser's own box. */
    window.HfsConfirm.ask(btn.dataset.confirm, { danger: true }).then(function (confirmed) {
      if (confirmed) remove(btn);
    });
  });

  function remove(btn) {
    var headers = { Accept: "application/fhir+json" };
    if (TENANT) headers["X-Tenant-ID"] = TENANT;
    /* The shared busy state (#679); the guard is per-button, so unrelated
       rows stay independently deletable. */
    window.hfsBusy.during([btn], function () {
      return fetch("/" + btn.dataset.type + "/" + btn.dataset.id, { method: "DELETE", headers: headers })
        .then(function (response) {
          if (!response.ok) throw new Error("HTTP " + response.status);
          /* #1240: this redirect is the delete's own navigation, not an
             abandoned edit — the unsaved-changes guard must not also ask. */
          if (window.HfsUnsaved) window.HfsUnsaved.suspend();
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
  }
})();
