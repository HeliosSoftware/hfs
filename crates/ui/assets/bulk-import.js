/* Page script for /ui/bulk-import/{id} (#1240): opt the detail page's Edit
   dialog — which edits a stored submission — into the shared unsaved-changes
   tracker, cued next to its own Cancel/Save row. `addbox.js` already asks
   before Cancel, Escape or an outside click discards a dirty dialog; nothing
   more is needed here. The New Submission dialog on /ui/bulk-import is a
   one-shot submit form, not a saved document, so it is deliberately left
   untracked. The dialogs stay fully usable without this script. */
(function () {
  "use strict";

  if (!window.HfsUnsaved) return;

  document
    .querySelectorAll("details.addbox--modal form[action$='/edit']")
    .forEach(function (form) {
      window.HfsUnsaved.track({ root: form, cue: form.querySelector(".addbox__actions") });
    });
})();
