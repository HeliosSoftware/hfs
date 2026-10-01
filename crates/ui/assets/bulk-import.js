/* Page script for /ui/bulk-import and /ui/bulk-import/{id} (#1240): opt
   every modal dialog's form on the page — New Submission and, on the detail
   page, Edit — into the shared unsaved-changes tracker, cued next to its own
   Cancel/Submit row. `addbox.js` already asks before Cancel, Escape or an
   outside click discards a dirty dialog; nothing more is needed here. The
   dialogs stay fully usable without this script. */
(function () {
  "use strict";

  if (!window.HfsUnsaved) return;

  document.querySelectorAll("details.addbox--modal form").forEach(function (form) {
    window.HfsUnsaved.track({ root: form, cue: form.querySelector(".addbox__actions") });
  });
})();
