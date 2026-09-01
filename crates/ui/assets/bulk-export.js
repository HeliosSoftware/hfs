/* Progressive enhancement for the Bulk Export builder (#792, #793). The
   server-rendered form remains usable without JavaScript: individual resource
   types and Custom instant stay enabled for native narrowing. */
(function () {
  "use strict";

  var form = document.querySelector("form.bulk-export-form");
  if (!form) return;

  var allTypes = form.querySelector('input[name="all_types"]');
  var types = Array.prototype.slice.call(form.querySelectorAll('input[name="types"]'));
  var nameInput = form.querySelector('input[name="name"]');
  var nameError = form.querySelector("#bulk-export-name-error");
  var sincePreset = form.querySelector('select[name="since_preset"]');
  var sinceCustom = form.querySelector('input[name="since_custom"]');
  var sinceCustomError = form.querySelector("#bulk-export-since-custom-error");
  var scopeRadios = Array.prototype.slice.call(form.querySelectorAll('input[name="scope"]'));
  var patientCombobox = form.querySelector(".combobox--scope-patient");
  var validationStarted = form.getAttribute("data-validation-started") === "true";

  function setFieldError(input, error, invalid) {
    if (!input || !error) return;

    var describedBy = (input.getAttribute("aria-describedby") || "")
      .split(/\s+/)
      .filter(Boolean)
      .filter(function (id) {
        return id !== error.id;
      });

    if (invalid) {
      input.setAttribute("aria-invalid", "true");
      describedBy.push(error.id);
      error.hidden = false;
    } else {
      input.removeAttribute("aria-invalid");
      error.hidden = true;
    }

    if (describedBy.length) {
      input.setAttribute("aria-describedby", describedBy.join(" "));
    } else {
      input.removeAttribute("aria-describedby");
    }
  }

  function validateName() {
    var invalid = Boolean(nameInput && !nameInput.value.trim());
    setFieldError(nameInput, nameError, invalid);
    return !invalid;
  }

  function isValidFhirInstant(value) {
    var pattern = sinceCustom && sinceCustom.getAttribute("data-pattern");
    if (!pattern || !new RegExp("^(?:" + pattern + ")$").test(value)) return false;

    var parts = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d+)?(Z|[+-](\d{2}):(\d{2}))$/.exec(
      value,
    );
    if (!parts) return false;

    var year = Number(parts[1]);
    var month = Number(parts[2]);
    var day = Number(parts[3]);
    var hour = Number(parts[4]);
    var minute = Number(parts[5]);
    var second = Number(parts[6]);
    if (year < 1 || month < 1 || month > 12 || hour > 23 || minute > 59 || second > 60) {
      return false;
    }

    var leapYear = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
    var daysInMonth = [31, leapYear ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if (day < 1 || day > daysInMonth[month - 1]) return false;

    if (parts[7] !== "Z") {
      var offsetHour = Number(parts[8]);
      var offsetMinute = Number(parts[9]);
      if (offsetMinute > 59 || offsetHour > 14 || (offsetHour === 14 && offsetMinute !== 0)) {
        return false;
      }
    }

    return true;
  }

  function validateSince() {
    var invalid = false;
    if (sincePreset && sinceCustom && sincePreset.value === "custom") {
      var value = sinceCustom.value.trim();
      invalid = Boolean(value && !isValidFhirInstant(value));
    }
    setFieldError(sinceCustom, sinceCustomError, invalid);
    return !invalid;
  }

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

  function synchronizePatientScope() {
    if (!patientCombobox) return;
    var patientScope = form.querySelector('input[name="scope"][value="patient"]');
    var active = Boolean(patientScope && patientScope.checked);
    var input = patientCombobox.querySelector('[role="combobox"]');
    if (input) input.disabled = !active;
    patientCombobox.querySelectorAll("[data-combobox-selected-input]").forEach(function (selected) {
      selected.disabled = !active;
    });
    if (!active) patientCombobox.dispatchEvent(new CustomEvent("hfs:combobox-close"));
  }

  // Browser-restored forms may come back with All Resources unchecked. Keep
  // their restored individual selections; only the default checked state
  // upgrades the grid to its checked-and-disabled presentation.
  synchronizeTypes(false);
  synchronizeSince();
  synchronizePatientScope();

  if (allTypes) {
    allTypes.addEventListener("change", function () {
      synchronizeTypes(!allTypes.checked);
    });
  }

  if (nameInput) {
    nameInput.addEventListener("input", function () {
      if (validationStarted) validateName();
    });
  }
  if (sincePreset) {
    sincePreset.addEventListener("change", function () {
      synchronizeSince();
      if (validationStarted) validateSince();
    });
  }
  if (sinceCustom) {
    sinceCustom.addEventListener("input", function () {
      if (validationStarted) validateSince();
    });
  }
  scopeRadios.forEach(function (scope) {
    scope.addEventListener("change", synchronizePatientScope);
  });
  if (patientCombobox) {
    patientCombobox.addEventListener("hfs:combobox-change", synchronizePatientScope);
  }

  form.addEventListener("submit", function (event) {
    validationStarted = true;
    var nameValid = validateName();
    var sinceValid = validateSince();
    if (nameValid && sinceValid) return;

    event.preventDefault();
    if (!nameValid && nameInput) {
      nameInput.focus();
    } else if (!sinceValid && sinceCustom) {
      sinceCustom.focus();
    }
  });

  // The reset event fires before native controls regain their default values.
  form.addEventListener("reset", function () {
    validationStarted = false;
    window.setTimeout(function () {
      synchronizeTypes(false);
      synchronizeSince();
      synchronizePatientScope();
      setFieldError(nameInput, nameError, false);
      setFieldError(sinceCustom, sinceCustomError, false);
    }, 0);
  });
})();
