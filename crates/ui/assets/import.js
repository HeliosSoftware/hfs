// HTS Import — paste/file toggle + drop zone with FileReader sink into the
// shared textarea.
//
// Wire contract mirrors the Batch page (crates/ui/assets/batch.js): the file
// content is read in the browser via FileReader.readAsText() and written into
// the existing `#hts-import-bundle` textarea. The urlencoded form contract
// stays unchanged — the backend handler `import_run` still reads `bundle`
// from `application/x-www-form-urlencoded` and never sees `bundle_file`.
//
// Design doc §7.7: file support was scheduled for v1.5 and lands here without
// adding a Multipart extractor or any new dependency, keeping the "HTMX-only,
// no new tech" contract from the original spec.
//
// Caveat documented in the demo: urlencoding overhead is ~33%, so the effective
// JSON cap on the file path is ~7.5 MiB before HTS_MAX_BODY_SIZE (10 MiB) 413s.
// The drop zone enforces this client-side to give a clear message before 413.
(function () {
  "use strict";

  var MAX_FILE_SIZE = 7.5 * 1024 * 1024; // ~7.5 MiB

  var textarea = document.getElementById("hts-import-bundle");
  var fileInput = document.getElementById("hts-import-file");
  var radios = document.querySelectorAll('input[name="source"]');
  if (!textarea || !fileInput || radios.length === 0) return;

  var textareaField = textarea.closest(".field");
  var fileField = document.getElementById("hts-import-file-field") || fileInput.closest(".field");
  var drop = document.getElementById("hts-import-drop");
  var fileError = document.getElementById("hts-import-file-error");
  var fileSuccess = document.getElementById("hts-import-file-success");

  // i18n messages from data attributes on the file field
  var messages = fileField ? fileField.dataset : {};

  function currentMode() {
    for (var i = 0; i < radios.length; i++) {
      if (radios[i].checked) return radios[i].value;
    }
    return "paste";
  }

  function applyMode() {
    var mode = currentMode();
    var isFile = mode === "file";
    if (textareaField) textareaField.hidden = isFile;
    if (fileField) fileField.hidden = !isFile;
    // Do NOT set `textarea.disabled` here: HTML5 skips disabled inputs on
    // form submission, so the FileReader-populated value would never reach
    // the server and the pre-flight would 400 with "Paste a JSON Bundle
    // before submitting". The parent `.field` `hidden` toggle above is
    // enough to keep the textarea out of the user's way visually while
    // still letting its value ride along in the urlencoded body.
    textarea.readOnly = isFile;
  }

  radios.forEach(function (radio) {
    radio.addEventListener("change", applyMode);
  });

  // --- Feedback display ---

  function showError(message) {
    clearSuccess();
    if (fileError) {
      fileError.textContent = message;
      fileError.hidden = false;
    }
  }

  function clearError() {
    if (fileError) {
      fileError.hidden = true;
      fileError.textContent = "";
    }
  }

  function showSuccess(fileName) {
    clearError();
    if (fileSuccess) {
      var msg = messages.msgLoaded || "File loaded: {name}";
      fileSuccess.textContent = msg.replace("{name}", fileName);
      fileSuccess.hidden = false;
    }
  }

  function clearSuccess() {
    if (fileSuccess) {
      fileSuccess.hidden = true;
      fileSuccess.textContent = "";
    }
  }

  // --- File reading with validation ---

  function readFile(file) {
    clearError();
    clearSuccess();

    // Size check: urlencoding overhead ~33% means ~7.5 MiB effective cap
    if (file.size > MAX_FILE_SIZE) {
      showError(messages.msgTooLarge || "File exceeds the size limit.");
      return;
    }

    // Show reading indicator on the drop zone
    if (drop) {
      drop.setAttribute("aria-busy", "true");
      drop.classList.add("batch-drop--reading");
    }

    var reader = new FileReader();
    var fileName = file.name;

    reader.onload = function () {
      if (drop) {
        drop.removeAttribute("aria-busy");
        drop.classList.remove("batch-drop--reading");
      }
      textarea.value = typeof reader.result === "string" ? reader.result : "";
      showSuccess(fileName);
    };

    reader.onerror = reader.onabort = function () {
      if (drop) {
        drop.removeAttribute("aria-busy");
        drop.classList.remove("batch-drop--reading");
      }
      textarea.value = "";
      showError(messages.msgReadFailed || "The file could not be read.");
    };

    reader.readAsText(file);
  }

  // --- Drop zone handlers (mirrors batch.js) ---

  if (drop) {
    drop.addEventListener("click", function () {
      fileInput.click();
    });

    drop.addEventListener("dragover", function (e) {
      e.preventDefault();
      drop.classList.add("batch-drop--over");
    });

    drop.addEventListener("dragleave", function () {
      drop.classList.remove("batch-drop--over");
    });

    drop.addEventListener("drop", function (e) {
      e.preventDefault();
      drop.classList.remove("batch-drop--over");
      if (e.dataTransfer.files && e.dataTransfer.files[0]) {
        readFile(e.dataTransfer.files[0]);
      }
    });
  }

  // --- File input change handler (also used by drop zone click path) ---

  fileInput.addEventListener("change", function () {
    var file = fileInput.files && fileInput.files[0];
    if (!file) return;
    readFile(file);
  });

  applyMode();
})();
