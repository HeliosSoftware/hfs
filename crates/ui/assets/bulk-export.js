/* Progressive enhancement for the Bulk Export builder (#792, #793). The
   server-rendered form remains usable without JavaScript: individual resource
   types and Custom instant stay enabled for native narrowing. */
(function () {
  "use strict";

  var form = document.querySelector("form.bulk-export-form");
  if (!form) return;

  var allTypes = form.querySelector('input[name="all_types"]');
  var types = Array.prototype.slice.call(form.querySelectorAll('input[name="types"]'));
  var sincePreset = form.querySelector('select[name="since_preset"]');
  var sinceCustom = form.querySelector('input[name="since_custom"]');

  function synchronizeTypes(clearIndividualTypes) {
    if (!allTypes) return;
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

  function synchronizeSince() {
    if (!sincePreset || !sinceCustom) return;
    sinceCustom.disabled = sincePreset.value !== "custom";
  }

  // Browser-restored forms may come back with All Resources unchecked. Keep
  // their restored individual selections; only the default checked state
  // upgrades the grid to its checked-and-disabled presentation.
  synchronizeTypes(false);
  synchronizeSince();

  if (allTypes) {
    allTypes.addEventListener("change", function () {
      synchronizeTypes(!allTypes.checked);
    });
  }

  if (sincePreset) sincePreset.addEventListener("change", synchronizeSince);

  // The reset event fires before native controls regain their default values.
  form.addEventListener("reset", function () {
    window.setTimeout(function () {
      synchronizeTypes(false);
      synchronizeSince();
    }, 0);
  });
})();
