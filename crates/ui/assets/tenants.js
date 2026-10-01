/* Page script for /ui/tenants. Two responsibilities:
   - #583: after a successful create the server answers with
     `HX-Trigger: tenant-created`; htmx dispatches that as a bubbling event on
     the form. Clear the form, collapse the add-tenant panel and hand focus
     back to its toggle. Failures carry no trigger, so the user's input
     survives alongside the error banner.
   - #582: while the user types a display name, mirror a slug of it into the
     tenant id, until they take the id over by editing it themselves.
   The page stays fully usable without this script. Provisioning itself now
   runs in the background (#581): the POST returns as soon as the server
   accepts the id, so there is nothing here left to hold the panel open for —
   the tenants table reports progress with its own polling row instead. */
(function () {
  "use strict";

  document.addEventListener("tenant-created", function (event) {
    var form = event.target && event.target.closest("form[hx-post='/ui/tenants']");
    if (!form) return;
    form.reset();
    var box = form.closest("details.addbox");
    if (!box) return;
    box.removeAttribute("open");
    var toggle = box.querySelector("summary");
    if (toggle) toggle.focus();
  });

  /* Slug mirror (#582): while the user types a display name, keep the tenant
     id in step with a slug of it, until they take the id over by editing it.
     The slug is a UI convention, deliberately narrower than what the server
     accepts (TenantId::parse: ASCII letters/digits, `-`, `_`, `.`, `/`, case
     preserved): lowercase, ASCII letters/digits only, runs of anything else
     collapsed to one `-`, no leading/trailing `-`, at most 64 bytes, and never
     a reserved segment. Typing `/`, `.` or `_` by hand still works — the
     mirror just never produces a hierarchy by accident. */
  var RESERVED = ["tenants", "resources", "history", "bulk"]; // RESERVED_TENANT_SEGMENTS (persistence/src/tenant/id.rs)
  var RESERVED_PREFIXES = ["__system__", "_system."];

  function slugify(name) {
    var slug = name
      .normalize("NFKD")
      .replace(/[\u0300-\u036f]/g, "")
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "");
    if (slug.length > 64) slug = slug.slice(0, 64).replace(/-+$/g, "");
    var reserved =
      RESERVED.indexOf(slug) !== -1 ||
      RESERVED_PREFIXES.some(function (p) { return slug.indexOf(p) === 0; });
    return reserved ? slug + "-tenant" : slug;
  }

  var form = document.querySelector("form[hx-post='/ui/tenants']");
  var nameInput = form && form.querySelector("[data-tenant-name]");
  var idInput = form && form.querySelector("[data-tenant-id]");
  if (form && nameInput && idInput) {
    // True once the user has typed into the id themselves; a restored value
    // that does not match the mirrored slug counts as theirs too.
    var userOwnsId = idInput.value !== "" && idInput.value !== slugify(nameInput.value);

    nameInput.addEventListener("input", function () {
      if (!userOwnsId) idInput.value = slugify(nameInput.value);
    });
    idInput.addEventListener("input", function () {
      // Clearing the id hands it back to the mirror.
      userOwnsId = idInput.value !== "";
    });
    form.addEventListener("reset", function () {
      userOwnsId = false;
    });
  }
})();
