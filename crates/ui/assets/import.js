// HTS Import — paste/file toggle + FileReader sink into the shared textarea.
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
(function () {
  "use strict";

  var textarea = document.getElementById("hts-import-bundle");
  var fileInput = document.getElementById("hts-import-file");
  var radios = document.querySelectorAll('input[name="source"]');
  if (!textarea || !fileInput || radios.length === 0) return;

  var textareaField = textarea.closest(".field");
  var fileField = fileInput.closest(".field");

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
    textarea.disabled = isFile;
  }

  radios.forEach(function (radio) {
    radio.addEventListener("change", applyMode);
  });

  fileInput.addEventListener("change", function () {
    var file = fileInput.files && fileInput.files[0];
    if (!file) return;
    var reader = new FileReader();
    reader.onload = function () {
      textarea.value = typeof reader.result === "string" ? reader.result : "";
    };
    reader.onerror = function () {
      textarea.value = "";
    };
    reader.readAsText(file);
  });

  applyMode();
})();
