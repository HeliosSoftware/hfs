/* Progressive enhancement for the Bulk Export builder (#792, #793, #1016).
   The server-rendered form remains usable without JavaScript: individual
   resource types and Custom instant stay enabled for native narrowing, and
   the Patients scope is rejected server-side when no patient was chosen.
   With JavaScript that same check also runs inline before submit. */
(function () {
  "use strict";

  var form = document.querySelector("form.bulk-export-form");
  if (!form) return;

  var allTypes = form.querySelector('input[name="all_types"]');
  var types = Array.prototype.slice.call(form.querySelectorAll('input[name="types"]'));
  var nameInput = form.querySelector('input[name="name"]');
  var nameHeading = document.querySelector("[data-bulk-export-name-heading]");
  var defaultHeading = nameHeading ? nameHeading.textContent : "";
  var nameError = form.querySelector("#bulk-export-name-error");
  var sincePreset = form.querySelector('select[name="since_preset"]');
  var sinceCustom = form.querySelector('input[name="since_custom"]');
  var sinceCustomError = form.querySelector("#bulk-export-since-custom-error");
  var untilInput = form.querySelector('input[name="until"]');
  var untilError = form.querySelector("#bulk-export-until-error");
  var scopeRadios = Array.prototype.slice.call(form.querySelectorAll('input[name="scope"]'));
  var patientCombobox = form.querySelector(".combobox--scope-patient");
  var patientsError = form.querySelector("#bulk-export-patients-error");
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

  function synchronizeName() {
    if (!nameInput || !nameHeading) return;
    nameHeading.textContent = nameInput.value.trim() || defaultHeading;
  }

  function isValidFhirInstant(value, pattern) {
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
      var pattern = sinceCustom.getAttribute("data-pattern");
      invalid = Boolean(value && !isValidFhirInstant(value, pattern));
    }
    setFieldError(sinceCustom, sinceCustomError, invalid);
    return !invalid;
  }

  // Lower bound Since resolves to, in epoch milliseconds, or NaN when it is
  // open or not yet valid. Mirrors the server's preset arithmetic.
  function sinceBound() {
    if (!sincePreset) return NaN;
    var days = { day: 1, week: 7, month: 28 }[sincePreset.value];
    if (days) return Date.now() - days * 86400000;
    if (sincePreset.value !== "custom" || !sinceCustom) return NaN;
    var value = sinceCustom.value.trim();
    if (!value || !isValidFhirInstant(value, sinceCustom.getAttribute("data-pattern"))) return NaN;
    return Date.parse(value);
  }

  // Until has no preset: it is validated whenever it is non-empty, and must
  // not fall before Since, which would make the export window empty (#1271).
  function validateUntil() {
    var invalid = false;
    var message = untilError && untilError.getAttribute("data-invalid-message");
    if (untilInput) {
      var value = untilInput.value.trim();
      var pattern = untilInput.getAttribute("data-pattern");
      if (value && !isValidFhirInstant(value, pattern)) {
        invalid = true;
      } else if (value && Date.parse(value) < sinceBound()) {
        invalid = true;
        message = untilError && untilError.getAttribute("data-order-message");
      }
    }
    if (invalid && message) untilError.textContent = message;
    setFieldError(untilInput, untilError, invalid);
    return !invalid;
  }

  var submitButton = form.querySelector('button[type="submit"]');
  var releaseSubmitBusy = null;
  var submitAttempt = 0;

  function prefetchNavigationAsset(href, as) {
    var link = document.createElement("link");
    if (link.relList && link.relList.supports && !link.relList.supports("prefetch")) {
      return Promise.resolve();
    }

    return new Promise(function (resolve) {
      var settled = false;
      var timeout = window.setTimeout(settle, 1000);

      function settle() {
        if (settled) return;
        settled = true;
        window.clearTimeout(timeout);
        link.removeEventListener("load", settle);
        link.removeEventListener("error", settle);
        resolve();
      }

      link.rel = "prefetch";
      link.href = href;
      link.as = as;
      link.addEventListener("load", settle);
      link.addEventListener("error", settle);
      document.head.appendChild(link);
    });
  }

  function prefetchNavigationAssets() {
    return Promise.all([
      prefetchNavigationAsset("/ui/assets/app.css", "style"),
      prefetchNavigationAsset("/ui/assets/theme.js", "script"),
    ]);
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

  function patientScopeSelected() {
    var patientScope = form.querySelector('input[name="scope"][value="patient"]');
    return Boolean(patientScope && patientScope.checked);
  }

  function synchronizePatientScope() {
    if (!patientCombobox) return;
    var active = patientScopeSelected();
    var input = patientCombobox.querySelector('[role="combobox"]');
    if (input) input.disabled = !active;
    patientCombobox.querySelectorAll("[data-combobox-selected-input]").forEach(function (selected) {
      selected.disabled = !active;
    });
    if (!active) patientCombobox.dispatchEvent(new CustomEvent("hfs:combobox-close"));
  }

  function patientField() {
    if (!patientCombobox) return null;
    var enhancement = patientCombobox.querySelector("[data-combobox-enhancement]");
    if (enhancement && !enhancement.hidden) {
      return patientCombobox.querySelector('[role="combobox"]');
    }
    return patientCombobox.querySelector('textarea[name="patient"]');
  }

  function hasPatientSelection() {
    if (!patientCombobox) return false;
    var enhancement = patientCombobox.querySelector("[data-combobox-enhancement]");
    if (enhancement && !enhancement.hidden) {
      return patientCombobox.querySelectorAll("[data-combobox-selected-input]").length > 0;
    }
    var fallback = patientCombobox.querySelector('textarea[name="patient"]');
    return Boolean(fallback && /[^\s,]/.test(fallback.value));
  }

  function validatePatients() {
    var invalid = Boolean(
      patientCombobox && patientsError && patientScopeSelected() && !hasPatientSelection(),
    );
    setFieldError(patientField(), patientsError, invalid);
    return !invalid;
  }

  // Browser-restored forms may come back with All Resources unchecked. Keep
  // their restored individual selections; only the default checked state
  // upgrades the grid to its checked-and-disabled presentation.
  synchronizeTypes(false);
  synchronizeName();
  synchronizeSince();
  synchronizePatientScope();

  // #1240: opt this form into the shared unsaved-changes tracker, cued next
  // to the Start button — captured only now, after the sync above: it can
  // check every individual type box (All Resources checked), and a
  // baseline taken before that would forever disagree with the very state
  // the page just loaded into.
  var unsaved = window.HfsUnsaved
    ? window.HfsUnsaved.track({ root: form, cue: form.querySelector(".form-actions") })
    : null;

  if (allTypes) {
    allTypes.addEventListener("change", function () {
      synchronizeTypes(!allTypes.checked);
    });
  }

  if (nameInput) {
    nameInput.addEventListener("input", function () {
      synchronizeName();
      if (validationStarted) validateName();
    });
  }
  if (sincePreset) {
    sincePreset.addEventListener("change", function () {
      synchronizeSince();
      if (validationStarted) {
        validateSince();
        validateUntil();
      }
    });
  }
  if (sinceCustom) {
    sinceCustom.addEventListener("input", function () {
      if (validationStarted) {
        validateSince();
        validateUntil();
      }
    });
  }
  if (untilInput) {
    untilInput.addEventListener("input", function () {
      if (validationStarted) validateUntil();
    });
  }
  scopeRadios.forEach(function (scope) {
    scope.addEventListener("change", function () {
      synchronizePatientScope();
      if (validationStarted) validatePatients();
    });
  });
  if (patientCombobox) {
    patientCombobox.addEventListener("hfs:combobox-change", function () {
      synchronizePatientScope();
      if (validationStarted) validatePatients();
    });
  }

  form.addEventListener("submit", function (event) {
    validationStarted = true;
    var nameValid = validateName();
    var sinceValid = validateSince();
    var untilValid = validateUntil();
    var patientsValid = validatePatients();
    if (!nameValid || !sinceValid || !untilValid || !patientsValid) {
      event.preventDefault();
      if (!nameValid && nameInput) {
        nameInput.focus();
      } else if (!sinceValid && sinceCustom) {
        sinceCustom.focus();
      } else if (!untilValid && untilInput) {
        untilInput.focus();
      } else if (!patientsValid) {
        var field = patientField();
        if (field) field.focus();
      }
      return;
    }

    if (!submitButton || !window.hfsBusy) return;

    event.preventDefault();
    window.hfsBusy.during([submitButton], function () {
      submitAttempt += 1;
      var currentAttempt = submitAttempt;

      prefetchNavigationAssets().then(function () {
        if (currentAttempt !== submitAttempt) return;
        // #1240: `HTMLFormElement.prototype.submit` never fires a `submit`
        // event, so unsaved.js's own document-level listener (which
        // suspends the browser guard for a form it tracks) never sees this
        // navigation — suspend it here instead, or a valid Start would
        // still trigger the "leave site?" prompt.
        if (unsaved) window.HfsUnsaved.suspend();
        HTMLFormElement.prototype.submit.call(form);
      });

      // Navigation normally discards this document. A bfcache restore keeps
      // it alive, so pageshow below releases the state in that one case.
      return new Promise(function (resolve) {
        releaseSubmitBusy = resolve;
      });
    });
  });

  window.addEventListener("pageshow", function (event) {
    if (!event.persisted || !releaseSubmitBusy) return;
    submitAttempt += 1;
    var release = releaseSubmitBusy;
    releaseSubmitBusy = null;
    release();
  });

  // The reset event fires before native controls regain their default values.
  form.addEventListener("reset", function () {
    validationStarted = false;
    window.setTimeout(function () {
      synchronizeTypes(false);
      synchronizeName();
      synchronizeSince();
      synchronizePatientScope();
      setFieldError(nameInput, nameError, false);
      setFieldError(sinceCustom, sinceCustomError, false);
      setFieldError(untilInput, untilError, false);
    }, 0);
  });
})();
