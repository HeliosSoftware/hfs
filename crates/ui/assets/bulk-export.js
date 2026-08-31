/* Progressive enhancement for the Bulk Export resource-type controls (#792).
   The server-rendered form remains usable without JavaScript: All Resources is
   checked, while individual resource types stay enabled for native narrowing. */
(function () {
  "use strict";

  var form = document.querySelector("form.bulk-export-form");
  if (!form) return;

  var allTypes = form.querySelector('input[name="all_types"]');
  var types = Array.prototype.slice.call(form.querySelectorAll('input[name="types"]'));
  if (!allTypes) return;

  function synchronize(clearIndividualTypes) {
    types.forEach(function (type) {
      if (allTypes.checked) {
        type.checked = true;
        type.disabled = true;
      } else {
        type.disabled = false;
        if (clearIndividualTypes) type.checked = false;
      }
    });
  }

  // Browser-restored forms may come back with All Resources unchecked. Keep
  // their restored individual selections; only the default checked state
  // upgrades the grid to its checked-and-disabled presentation.
  synchronize(false);

  allTypes.addEventListener("change", function () {
    synchronize(!allTypes.checked);
  });

  // The reset event fires before native controls regain their default values.
  form.addEventListener("reset", function () {
    window.setTimeout(function () {
      synchronize(false);
    }, 0);
  });
})();
