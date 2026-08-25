/*
 * Batch / Transaction workspace (#476, Brett's frames): pick a Bundle JSON,
 * review the execution plan (one row per entry, method chip, collapsible
 * body), execute against the FHIR root, and read the per-action outcomes plus
 * the aggregate result. The bundle's own `type` decides the semantics copy.
 */
(function () {
  "use strict";

  var root = document.getElementById("batch");
  if (!root || !window.fetch) return;
  var messages = root.dataset;

  /* The effective tenant, stamped by the server (#344); FHIR calls carry it. */
  var TENANT = (document.querySelector('meta[name="hfs-tenant"]') || {}).content || "";
  function fhirHeaders(extra) {
    var h = { Accept: "application/fhir+json" };
    if (TENANT) h["X-Tenant-ID"] = TENANT;
    if (extra) for (var k in extra) h[k] = extra[k];
    return h;
  }

  var stages = {
    upload: document.getElementById("batch-upload"),
    preflight: document.getElementById("batch-preflight"),
    response: document.getElementById("batch-response"),
  };
  function show(stage) {
    for (var k in stages) stages[k].hidden = k !== stage;
  }

  var drop = document.getElementById("batch-drop");
  var fileInput = document.getElementById("batch-file");
  var uploadError = document.getElementById("batch-upload-error");
  var requestLine = document.getElementById("batch-request-line");
  var semantics = document.getElementById("batch-semantics");
  var rows = document.getElementById("batch-rows");
  var rawJson = document.getElementById("batch-json");
  var tabActions = document.getElementById("batch-tab-actions");
  var tabJson = document.getElementById("batch-tab-json");
  var executeError = document.getElementById("batch-execute-error");
  var outcomes = document.getElementById("batch-outcomes");
  var overall = document.getElementById("batch-overall");
  var summary = document.getElementById("batch-summary");

  var bundle = null;

  /* ---- stage 1: pick the file ---------------------------------------- */

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
    if (e.dataTransfer.files && e.dataTransfer.files[0]) readFile(e.dataTransfer.files[0]);
  });
  fileInput.addEventListener("change", function () {
    if (fileInput.files && fileInput.files[0]) readFile(fileInput.files[0]);
  });

  function fail(message) {
    uploadError.textContent = message;
    uploadError.hidden = false;
  }

  function readFile(file) {
    uploadError.hidden = true;
    var reader = new FileReader();
    reader.onload = function () {
      var parsed;
      try {
        parsed = JSON.parse(reader.result);
      } catch (e) {
        return fail(messages.msgInvalidJson + " (" + e.message + ")");
      }
      if (!parsed || parsed.resourceType !== "Bundle") return fail(messages.msgNotABundle);
      if (parsed.type !== "batch" && parsed.type !== "transaction") return fail(messages.msgBadType);
      bundle = parsed;
      renderPreflight();
      show("preflight");
    };
    reader.readAsText(file);
  }

  /* ---- stage 2: the execution plan ------------------------------------ */

  function entries() {
    return Array.isArray(bundle.entry) ? bundle.entry : [];
  }

  function methodOf(entry) {
    return ((entry.request && entry.request.method) || "?").toUpperCase();
  }

  function renderPreflight() {
    var n = entries().length;
    requestLine.textContent =
      "POST [base] · Bundle · " + bundle.type + " · " + n + " " + messages.msgEntries;
    semantics.textContent =
      bundle.type === "transaction" ? messages.msgSemanticsTransaction : messages.msgSemanticsBatch;

    rows.textContent = "";
    entries().forEach(function (entry, i) {
      var li = document.createElement("li");
      li.className = "batch-row";

      var head = document.createElement("button");
      head.type = "button";
      head.className = "batch-row__head";
      head.setAttribute("aria-expanded", "false");

      var num = document.createElement("span");
      num.className = "batch-row__num";
      num.textContent = String(i + 1);

      var method = methodOf(entry);
      var chip = document.createElement("span");
      chip.className = "batch-chip batch-chip--" + method.toLowerCase();
      chip.textContent = method;

      var url = document.createElement("code");
      url.className = "batch-row__url";
      url.textContent = (entry.request && entry.request.url) || "";

      var arrow = document.createElement("span");
      arrow.className = "batch-row__arrow";
      /* The app's chevron icon, template-rendered so the SVG stays vendored
         in templates/icons rather than inlined here (#674). */
      var chevron = document.getElementById("batch-chevron");
      if (chevron) arrow.appendChild(chevron.content.cloneNode(true));

      head.appendChild(num);
      head.appendChild(chip);
      head.appendChild(url);
      head.appendChild(arrow);

      var body = document.createElement("pre");
      body.className = "batch-row__body";
      body.hidden = true;
      body.textContent = entry.resource
        ? JSON.stringify(entry.resource, null, 2)
        : messages.msgNoBody;

      head.addEventListener("click", function () {
        body.hidden = !body.hidden;
        head.setAttribute("aria-expanded", body.hidden ? "false" : "true");
      });

      li.appendChild(head);
      li.appendChild(body);
      rows.appendChild(li);
    });

    rawJson.textContent = JSON.stringify(bundle, null, 2);
    selectTab("actions");
  }

  function selectTab(which) {
    var actions = which === "actions";
    tabActions.setAttribute("aria-selected", actions ? "true" : "false");
    tabJson.setAttribute("aria-selected", actions ? "false" : "true");
    rows.hidden = !actions;
    rawJson.hidden = actions;
  }
  tabActions.addEventListener("click", function () { selectTab("actions"); });
  tabJson.addEventListener("click", function () { selectTab("json"); });

  function reset() {
    bundle = null;
    fileInput.value = "";
    show("upload");
  }
  document.getElementById("batch-cancel").addEventListener("click", reset);
  document.getElementById("batch-cancel-top").addEventListener("click", reset);

  /* ---- stage 3: execute and report ------------------------------------ */

  function execute() {
    executeError.hidden = true;
    fetch("/", {
      method: "POST",
      headers: fhirHeaders({ "Content-Type": "application/fhir+json" }),
      credentials: "same-origin",
      body: JSON.stringify(bundle),
    })
      .then(function (response) {
        return response
          .json()
          .catch(function () { return null; })
          .then(function (body) { renderResponse(response, body); });
      })
      .catch(function (e) {
        executeError.textContent = messages.msgRequestFailed + " (" + e.message + ")";
        executeError.hidden = false;
      });
  }
  document.getElementById("batch-execute").addEventListener("click", execute);
  document.getElementById("batch-execute-top").addEventListener("click", execute);
  /* The run already happened: Done lands back on a clean upload stage
     rather than offering to re-run a mutation that succeeded (#675). */
  document.getElementById("batch-done").addEventListener("click", reset);

  function renderResponse(response, body) {
    overall.textContent = String(response.status);
    overall.className = "batch-badge " + (response.ok ? "batch-badge--ok" : "batch-badge--error");

    outcomes.textContent = "";
    var created = 0, updated = 0, other = 0, failed = 0;
    var responded = (body && Array.isArray(body.entry)) ? body.entry : [];

    // The whole-bundle failure case (e.g. a rolled-back transaction): show
    // the OperationOutcome text instead of pretending there are outcomes.
    if (!response.ok && (!responded.length || (body && body.resourceType === "OperationOutcome"))) {
      var diag = "";
      if (body && body.issue && body.issue[0]) {
        diag = body.issue[0].diagnostics || (body.issue[0].details && body.issue[0].details.text) || "";
      }
      executeError.textContent = messages.msgRequestFailed + (diag ? " — " + diag : "");
      executeError.hidden = false;
      return;
    }

    responded.forEach(function (entry, i) {
      var status = (entry.response && entry.response.status) || "";
      var request = entries()[i] || {};
      var li = document.createElement("li");
      li.className = "batch-row";
      var head = document.createElement("div");
      head.className = "batch-row__head batch-row__head--static";

      var num = document.createElement("span");
      num.className = "batch-row__num";
      num.textContent = String(i + 1);
      var method = methodOf(request);
      var chip = document.createElement("span");
      chip.className = "batch-chip batch-chip--" + method.toLowerCase();
      chip.textContent = method;
      var url = document.createElement("code");
      url.className = "batch-row__url";
      url.textContent = (request.request && request.request.url) || "";
      var badge = document.createElement("span");
      var code = parseInt(status, 10) || 0;
      var ok = code >= 200 && code < 300;
      badge.className = "batch-badge " + (ok ? "batch-badge--ok" : "batch-badge--error");
      badge.textContent = status;

      if (!ok) failed++;
      else if (code === 201) created++;
      else if (method === "PUT" || method === "PATCH") updated++;
      else other++;

      head.appendChild(num);
      head.appendChild(chip);
      head.appendChild(url);
      head.appendChild(badge);
      li.appendChild(head);
      outcomes.appendChild(li);
    });

    var parts = [];
    if (created) parts.push(created + " " + messages.msgCreated);
    if (updated) parts.push(updated + " " + messages.msgUpdated);
    if (other) parts.push(other + " " + messages.msgOther);
    if (failed) parts.push(failed + " " + messages.msgFailed);
    summary.textContent = parts.join(" · ");

    show("response");
  }
})();
