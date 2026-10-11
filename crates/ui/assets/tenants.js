/* Page script for /ui/tenants. Four responsibilities:
   - #583: after a successful create the server answers with
     `HX-Trigger: tenant-created`; htmx dispatches that as a bubbling event on
     the form. Clear the form, collapse the add-tenant panel and hand focus
     back to its toggle. Failures carry no trigger, so the user's input
     survives alongside the error banner.
   - #582: while the user types a display name, mirror a slug of it into the
     tenant id, until they take the id over by editing it themselves.
   - #1851: issue the row deletes (and dismissals) from an element that
     outlives the rows, so a delete queued behind another request is sent.
   - #1851: keep the count status line's live region as it is while its
     words do not change, so assistive technology does not repeat it.
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

  /* Row deletes (#1851). Every request that swaps #tenant-rows queues on the
     table card (hx-sync), and htmx issues a queued request later from the
     element that asked for it, skipping it without a word if that element
     has left the page. A trash button lives inside #tenant-rows, which the
     in-flight response replaces, so a delete confirmed while a poll, search,
     create or other delete was in flight would never be sent. The buttons
     are therefore not htmx elements: this confirms (in-page, #1667), then
     issues the DELETE from [data-tenant-mutations], which stays put and
     carries the target, the swap, the search term and the sync. */
  document.addEventListener("click", function (event) {
    var button = event.target && event.target.closest ? event.target.closest("button[data-tenant-delete]") : null;
    if (!button || !window.htmx) return;
    var issuer = document.querySelector("[data-tenant-mutations]");
    if (!issuer) return;
    event.preventDefault();
    var path = button.getAttribute("data-tenant-delete");
    var question = button.getAttribute("data-confirm");
    var asked = question && window.HfsConfirm
      ? window.HfsConfirm.ask(question, { danger: true })
      : Promise.resolve(true);
    asked.then(function (confirmed) {
      if (!confirmed) return;
      // htmx reports a failed request through its own events and the
      // response's banner; the promise rejecting is not news here.
      window.htmx.ajax("DELETE", path, { source: issuer }).catch(function () {});
    });
  });

  /* Count status line (#1851). Every rows response carries
     #tenant-counts-status out of band, and replacing a role="status"
     region's nodes can make a screen reader say it again even when nothing
     changed: once per poll tick, once per search keystroke. While the state
     and the words are the same, keep the nodes and only follow the Refresh
     link's search term. */
  document.addEventListener("htmx:oobBeforeSwap", function (event) {
    var detail = event.detail || {};
    var target = detail.target;
    var incoming = detail.fragment;
    if (!target || target.id !== "tenant-counts-status" || !incoming || !incoming.querySelector) return;
    var now = target.querySelector("[data-counts-state]");
    var next = incoming.querySelector("[data-counts-state]");
    if (!now || !next) return;
    if (now.getAttribute("data-counts-state") !== next.getAttribute("data-counts-state")) return;
    if (now.textContent !== next.textContent) return;
    var nowLink = target.querySelector("a.counts-status__refresh");
    var nextLink = incoming.querySelector("a.counts-status__refresh");
    if (!nowLink !== !nextLink) return;
    if (nowLink) nowLink.setAttribute("href", nextLink.getAttribute("href"));
    detail.shouldSwap = false;
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
