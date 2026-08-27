/*
 * Batch / Transaction workspace (#476, Brett's frames): pick a Bundle JSON,
 * review the execution plan (one row per entry, method chip, collapsible
 * body), execute against the FHIR root, and read the per-action outcomes plus
 * the aggregate result. The bundle's own `type` decides the semantics copy.
 */
(function () {
  "use strict";

  var root = document.getElementById("batch");
  if (!root || !window.fetch || !window.hfsBusy) return;
  var messages = root.dataset;
  var hfsBusy = window.hfsBusy;

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
  var executeTopBtn = document.getElementById("batch-execute-top");
  var cancelTopBtn = document.getElementById("batch-cancel-top");
  var createdBadge = document.getElementById("batch-created");
  var doneBtn = document.getElementById("batch-done");
  var busyRegion = document.getElementById("batch-busy");

  var bundle = null;
  var bundleJsonRenderer = null;
  var renderGeneration = 0;
  var renderControllers = [];

  function resetJsonRendering() {
    renderGeneration++;
    renderControllers.forEach(function (controller) { controller.abort(); });
    renderControllers = [];
    return renderGeneration;
  }

  function fallbackJson(target, value) {
    target.textContent = "";
    var pre = document.createElement("pre");
    pre.className = "json-view__fallback";
    pre.textContent = JSON.stringify(value, null, 2);
    target.appendChild(pre);
  }

  function lazyJson(target, value, generation) {
    var state = "idle";
    return function () {
      if (state === "pending" || state === "ready" || state === "fallback" || generation !== renderGeneration) return;
      state = "pending";
      var controller = new AbortController();
      renderControllers.push(controller);
      fetch("/ui/json-view/render", {
        method: "POST",
        headers: { "Content-Type": "application/json", Accept: "text/html" },
        credentials: "same-origin",
        body: JSON.stringify(value),
        signal: controller.signal,
      })
        .then(function (response) {
          if (!response.ok) throw new Error(String(response.status));
          return response.text();
        })
        .then(function (html) {
          if (generation !== renderGeneration || !target.isConnected) return;
          target.innerHTML = html;
          state = "ready";
        })
        .catch(function (error) {
          if (generation !== renderGeneration || error.name === "AbortError") return;
          fallbackJson(target, value);
          state = "fallback";
        })
        .finally(function () {
          var index = renderControllers.indexOf(controller);
          if (index !== -1) renderControllers.splice(index, 1);
        });
    };
  }

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

  function clearPreflight() {
    rows.textContent = "";
    rawJson.textContent = "";
    bundleJsonRenderer = null;
  }

  /* Yield so the busy region paints before a heavy parse blocks the main
     thread; rAF never fires in a hidden tab, hence the timeout lane. */
  function afterPaint(fn) {
    if (document.hidden) {
      setTimeout(fn, 0);
      return;
    }
    requestAnimationFrame(function () {
      setTimeout(fn, 0);
    });
  }

  function readFile(file) {
    var generation = resetJsonRendering();
    bundle = null;
    clearPreflight();
    show("upload");
    uploadError.hidden = true;
    /* Busy is up before the file is even read (#679): the region reveal is
       synchronous, FileReader delivery is already async. */
    var busy = hfsBusy.region(busyRegion, messages.msgReading);
    var reader = new FileReader();
    /* A folder drop or a file that vanished between pick and read fires
       error/abort, never load — the region must clear and say something. */
    reader.onerror = reader.onabort = function () {
      if (generation !== renderGeneration) return;
      try {
        fail(messages.msgReadFailed);
      } finally {
        busy.done();
      }
    };
    reader.onload = function () {
      if (generation !== renderGeneration) return;
      afterPaint(function () {
        /* Re-checked here, not just above: a second pick during this
           deferred window owns the stage now. */
        if (generation !== renderGeneration) return;
        try {
          var parsed;
          try {
            parsed = JSON.parse(reader.result);
          } catch (e) {
            return fail(messages.msgInvalidJson + " (" + e.message + ")");
          }
          if (!parsed || parsed.resourceType !== "Bundle") return fail(messages.msgNotABundle);
          if (parsed.type !== "batch" && parsed.type !== "transaction") return fail(messages.msgBadType);
          bundle = parsed;
          renderPreflight(generation);
          show("preflight");
          /* The drop zone was hidden with its stage, stranding focus.
             Checked by containment, not activeElement === body: the focus
             fixup for a hidden element runs at the next render update, so
             the hidden trigger can still read as active here. Land on the
             revealed stage, not its primary action. */
          var active = document.activeElement;
          if (active === document.body || stages.upload.contains(active)) {
            /* preventScroll (#732): on a plan taller than the viewport the
               default scroll-into-view pinned the Execute row to the top. */
            stages.preflight.focus({ preventScroll: true });
          }
        } finally {
          busy.done();
        }
      });
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

  function renderPreflight(generation) {
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

      var body = document.createElement("div");
      body.className = "batch-row__body";
      body.hidden = true;
      var showJson = null;
      if (entry.resource) {
        showJson = lazyJson(body, entry.resource, generation);
      } else {
        var noBody = document.createElement("pre");
        noBody.className = "json-view__fallback";
        noBody.textContent = messages.msgNoBody;
        body.appendChild(noBody);
      }

      head.addEventListener("click", function () {
        body.hidden = !body.hidden;
        head.setAttribute("aria-expanded", body.hidden ? "false" : "true");
        if (!body.hidden && showJson) showJson();
      });

      li.appendChild(head);
      li.appendChild(body);
      rows.appendChild(li);
    });

    rawJson.textContent = "";
    bundleJsonRenderer = lazyJson(rawJson, bundle, generation);
    selectTab("actions");
  }

  function selectTab(which) {
    var actions = which === "actions";
    tabActions.setAttribute("aria-selected", actions ? "true" : "false");
    tabJson.setAttribute("aria-selected", actions ? "false" : "true");
    rows.hidden = !actions;
    rawJson.hidden = actions;
    if (!actions && bundleJsonRenderer) bundleJsonRenderer();
  }
  tabActions.addEventListener("click", function () { selectTab("actions"); });
  tabJson.addEventListener("click", function () { selectTab("json"); });

  function reset() {
    resetJsonRendering();
    bundle = null;
    fileInput.value = "";
    clearPreflight();
    /* Errors are page-level elements, not stage content: hiding a stage
       leaves them set, so an abandoned attempt's error would greet the next
       one (#731). */
    uploadError.hidden = true;
    executeError.hidden = true;
    show("upload");
    /* Done/Cancel hid the stage that held focus; land on the one action
       the upload stage offers (#679). Containment, not body: the focus
       fixup for the hidden trigger runs at the next render update. */
    var active = document.activeElement;
    if (
      active === document.body ||
      stages.preflight.contains(active) ||
      stages.response.contains(active)
    ) {
      drop.focus();
    }
  }
  cancelTopBtn.addEventListener("click", reset);

  /* ---- stage 3: execute and report ------------------------------------ */

  function execute() {
    executeError.hidden = true;
    if (!bundle) {
      fail(messages.msgInvalidJson);
      show("upload");
      return;
    }
    /* The whole footer goes inert (#679): both Execute copies spin, and the
       Cancels disable with them — a mid-flight Cancel nulled `bundle` and
       crashed the settling renderResponse. Busy holds until the outcome is
       rendered, not merely until response headers arrive. */
    hfsBusy.during(
      [executeTopBtn],
      function () {
        return fetch("/", {
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
      },
      { alsoDisable: [cancelTopBtn], region: busyRegion, label: messages.msgExecuting }
    );
  }
  executeTopBtn.addEventListener("click", execute);
  /* The run already happened: Done lands back on a clean upload stage
     rather than offering to re-run a mutation that succeeded (#675). */
  doneBtn.addEventListener("click", reset);

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

    /* The created count reads in the card head next to the status badge
       (#729); the rest of the tally keeps the summary line, failures above
       all. */
    createdBadge.textContent = created ? created + " " + messages.msgCreated : "";
    var parts = [];
    if (updated) parts.push(updated + " " + messages.msgUpdated);
    if (other) parts.push(other + " " + messages.msgOther);
    if (failed) parts.push(failed + " " + messages.msgFailed);
    summary.textContent = parts.join(" · ");
    summary.hidden = !parts.length;

    show("response");
    /* The disabled trigger was hidden with its stage; land on the one
       action this stage offers (#679). */
    doneBtn.focus();
  }
})();
