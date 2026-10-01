/* Progressive enhancement for the SQL Export builder (#834, #836, #837): a
   type switch, a text filter, a header select-all, and the "n of m
   selected" count over the subjects table; two independent #836 fields —
   the CSV header switch's visibility and the Since custom instant's
   enabled state and inline validation; and #837's per-SQL-Query values
   row — layered over markup that already renders plain and already
   submits without this script (the create form itself is #833's work;
   combobox.js, loaded alongside this file, owns the Patients/Groups
   pickers on its own terms).

   The one rule every piece of the subjects-table section honors: filtering
   never unchecks a row. A row the type switch or the text filter hides
   keeps whatever checked state it had — it still submits with the form — so
   searching for the next subject in a long list never costs the user a
   selection already made. The header select-all only ever acts on the rows
   currently visible; the count below the table always counts every checked
   box, visible or not.

   #837's values row (`tr.row--params`, right under a parameterized SQL
   Query subject's own row) layers three independent behaviors over the
   plain, always-open markup the server renders:

   - Visibility: a values row starts `hidden` unless its own query is
     checked; (un)checking the box (directly, or via the select-all) shows
     or hides it, always landing back on its expanded state — the values
     just typed are never cleared, only ignored while the query stays
     unmarked.
   - Fold: the chevron in the Subject cell (server-rendered `hidden`,
     revealed only for a checked query) toggles `aria-expanded` and, with
     it, whether the values row or its `.param-summary` chip strip is the
     one actually shown — the summary is rebuilt from the fields' live
     values on every fold and on every `input`/`change` while folded.
   - Filter interplay: hiding a subject's own row (the type switch/text
     filter above) hides its values row too, without touching its fold
     state, so unhiding it later restores exactly the view it had.
   - Missing values: the "n of m selected" hint gains a
     "· k value(s) missing" suffix counting empty, default-less fields
     across every *checked* query, recomputed on every relevant change; a
     submit with any such field blocks, opens the first affected row, and
     marks/focuses the field — the native `required` attribute is kept in
     sync with "checked and no default" throughout, so a plain browser
     submit (JavaScript absent, or this very check having already passed)
     can never be blocked by a query nobody selected.
   - Re-render with error: a values row the server sent back with
     `data-open` (a resubmission that left one of its fields invalid) opens
     on load and receives focus on its first invalid field, ahead of
     anything else on the page. */
(function () {
  "use strict";

  var form = document.querySelector("form.bulk-export-form");
  if (!form) return;

  // #837: once this script is active, it alone decides whether a submit
  // proceeds — native constraint validation is disabled on this form.
  // Browsers block an invalid submit *before* the `submit` event ever
  // fires, with no way for a listener to run its own check first; a
  // required field inside a folded (`hidden`) values row is still a
  // constraint-validation candidate (`hidden` on an ancestor is not one of
  // the "barred from constraint validation" conditions), so native
  // validation would otherwise block the submit *silently* — it has no
  // element to focus or scroll to, so nothing visible happens at all. The
  // script's own `blockSubmitOnMissingParams` (below) is what gives that
  // case real feedback: opening the row, marking the field, and focusing
  // it. This only matters once folding exists, which only exists with this
  // script running — the no-JavaScript fallback never sets `noValidate`,
  // so its own native `required` still blocks a plain submit exactly as
  // before, and every values row there is always visible, so native
  // validation always has a real field to focus.
  form.noValidate = true;

  // #836: the CSV header switch's visibility. The switch's own `checked`
  // state is never touched here — only whether its label (and the hint
  // right below it) is shown at all; a value set before switching away from
  // csv survives switching back. Independent of the subjects table below,
  // so it keeps working even if that table were ever empty.
  var formatInputs = Array.prototype.slice.call(form.querySelectorAll('input[name="format"]'));
  var headerCheckbox = form.querySelector('input[name="header"]');
  var headerLabel = headerCheckbox ? headerCheckbox.closest("label") : null;
  var headerHint = headerLabel ? headerLabel.nextElementSibling : null;

  function selectedFormat() {
    var checked = formatInputs.filter(function (input) {
      return input.checked;
    })[0];
    return checked ? checked.value : "";
  }

  function synchronizeHeaderVisibility() {
    if (!headerLabel) return;
    var isCsv = selectedFormat() === "csv";
    headerLabel.hidden = !isCsv;
    if (headerHint) headerHint.hidden = !isCsv;
  }

  formatInputs.forEach(function (input) {
    input.addEventListener("change", synchronizeHeaderVisibility);
  });
  synchronizeHeaderVisibility();

  // #836: Since's custom instant — enabled only for "custom", the same rule
  // Bulk Export's own Since field applies (bulk-export.js). Unlike that
  // field, the invalid check here is only the input's own `data-pattern`:
  // the fuller calendar-validity pass (leap days, day-of-month ranges) is
  // `crate::lookup::since_instant`'s job on the server; the browser only
  // needs to catch a shape no FHIR instant could ever have, and block the
  // submit on it.
  var sincePreset = form.querySelector('select[name="since_preset"]');
  var sinceCustom = form.querySelector('input[name="since_custom"]');
  var sinceCustomError = form.querySelector("#sql-export-since-custom-error");
  var sinceValidationStarted = false;

  function synchronizeSinceCustom() {
    if (!sincePreset || !sinceCustom) return;
    sinceCustom.disabled = sincePreset.value !== "custom";
  }

  function setSinceCustomInvalid(invalid) {
    if (!sinceCustom || !sinceCustomError) return;
    if (invalid) {
      sinceCustom.setAttribute("aria-invalid", "true");
      sinceCustom.setAttribute("aria-describedby", sinceCustomError.id);
      sinceCustomError.hidden = false;
    } else {
      sinceCustom.removeAttribute("aria-invalid");
      sinceCustom.removeAttribute("aria-describedby");
      sinceCustomError.hidden = true;
    }
  }

  function validateSinceCustom() {
    if (!sincePreset || !sinceCustom) return true;
    if (sincePreset.value !== "custom") {
      setSinceCustomInvalid(false);
      return true;
    }
    var value = sinceCustom.value.trim();
    var pattern = sinceCustom.getAttribute("data-pattern");
    var invalid = Boolean(value) && !(pattern && new RegExp("^(?:" + pattern + ")$").test(value));
    setSinceCustomInvalid(invalid);
    return !invalid;
  }

  synchronizeSinceCustom();
  if (sincePreset) {
    sincePreset.addEventListener("change", function () {
      synchronizeSinceCustom();
      if (sinceValidationStarted) validateSinceCustom();
    });
  }
  if (sinceCustom) {
    sinceCustom.addEventListener("input", function () {
      if (sinceValidationStarted) validateSinceCustom();
    });
  }

  var table = form.querySelector(".table-card");
  var tools = table && table.querySelector(".card-head__tools--subjects");
  var typeButtons = tools
    ? Array.prototype.slice.call(tools.querySelectorAll("[data-subject-filter]"))
    : [];
  var filterInput = tools ? tools.querySelector('input[type="search"]') : null;
  var selectAll = table && table.querySelector("thead .col-check input[type='checkbox']");
  var rows = table
    ? Array.prototype.slice.call(table.querySelectorAll("tbody tr[data-kind]"))
    : [];
  var emptyRow = table && table.querySelector("tbody tr.data-table__empty");
  var countHint = table && table.querySelector("[data-msg-count]");

  function rowCheckbox(row) {
    return row.querySelector('input[name="subject"]');
  }

  // ---------------------------------------------------------------------
  // #837: the values row under each parameterized SQL Query subject —
  // built unconditionally (empty when there is no table, or no
  // parameterized subject) so the merged submit listener below can always
  // rely on it, whatever the guarded, rows-only logic further down ends up
  // doing.
  // ---------------------------------------------------------------------

  var paramEntries = rows
    .map(function (row) {
      var next = row.nextElementSibling;
      var paramsRow = next && next.classList.contains("row--params") ? next : null;
      var toggle = row.querySelector(".row-toggle");
      var summary = row.querySelector(".param-summary");
      if (!paramsRow || !toggle || !summary) return null;
      return {
        row: row,
        box: rowCheckbox(row),
        paramsRow: paramsRow,
        toggle: toggle,
        summary: summary,
        fields: Array.prototype.slice.call(paramsRow.querySelectorAll(".field[data-param-name]")),
      };
    })
    .filter(Boolean);

  var missingOneTemplate = countHint ? countHint.dataset.msgMissingOne : "";
  var missingOtherTemplate = countHint ? countHint.dataset.msgMissingOther : "";
  var paramRequiredMessage = countHint ? countHint.dataset.msgParamRequired : "";
  var paramRequiredChip = countHint ? countHint.dataset.msgParamRequiredChip : "";

  function fieldControl(field) {
    return field.querySelector("input, select");
  }

  function fieldHasDefault(field) {
    return "default" in field.dataset;
  }

  function fieldValue(field) {
    var control = fieldControl(field);
    return control ? control.value.trim() : "";
  }

  /* Empty and no declared default: the one condition that keeps the server
     from ever binding a value for this field — exactly what both the
     missing-values count and the pre-submit block key off of. */
  function fieldIsMissing(field) {
    return !fieldValue(field) && !fieldHasDefault(field);
  }

  function entryMissingCount(entry) {
    if (!entry.box || !entry.box.checked) return 0;
    return entry.fields.filter(fieldIsMissing).length;
  }

  function totalMissingCount() {
    return paramEntries.reduce(function (total, entry) {
      return total + entryMissingCount(entry);
    }, 0);
  }

  function missingCountText(count) {
    var template = count === 1 ? missingOneTemplate : missingOtherTemplate;
    return template ? template.replace("{count}", String(count)) : "";
  }

  function expanded(entry) {
    return entry.toggle.getAttribute("aria-expanded") !== "false";
  }

  /* The native-validation half of the submit block below: a field only
     ever carries `required` while its query is checked and the field
     itself has no default — the same rule the server's own initial render
     applies (`ParamFieldView::required` in sql_export.rs) — kept true
     after every check/uncheck so a plain submit can never be blocked by a
     query nobody selected. */
  function synchronizeRequired(entry, checked) {
    entry.fields.forEach(function (field) {
      var control = fieldControl(field);
      if (!control) return;
      if (checked && !fieldHasDefault(field)) {
        control.setAttribute("required", "");
      } else {
        control.removeAttribute("required");
      }
    });
  }

  /* One `.tag--param`/`.tag--danger` chip per declared parameter: a value
     (typed, or the declared default when the field is empty) reads
     `:name = value`; a still-missing required field reads
     `:name — {required}` in the alert tone. */
  function renderSummary(entry) {
    entry.summary.textContent = "";
    entry.fields.forEach(function (field) {
      var chip = document.createElement("span");
      var value = fieldValue(field);
      if (!value && fieldHasDefault(field)) value = field.dataset["default"];
      if (value) {
        chip.className = "tag tag--param";
        chip.textContent = ":" + field.dataset.paramName + " = " + value;
      } else {
        chip.className = "tag tag--danger";
        chip.textContent =
          ":" + field.dataset.paramName + (paramRequiredChip ? " — " + paramRequiredChip : "");
      }
      entry.summary.appendChild(chip);
    });
  }

  /* The single place that reconciles one parameterized row's visibility:
     unchecked hides everything (chevron included — nothing left to
     toggle); checked shows the values row when expanded and visible under
     the current filter, or the chip summary when folded; either way the
     `required` attributes are re-synced first, so this is also the one
     function every checked/unchecked, fold, and filter change funnels
     through. */
  function sync(entry) {
    var checked = Boolean(entry.box && entry.box.checked);
    synchronizeRequired(entry, checked);
    entry.toggle.hidden = !checked;
    if (!checked) {
      entry.paramsRow.hidden = true;
      entry.summary.hidden = true;
      return;
    }
    var filterHidden = entry.row.hidden;
    var isExpanded = expanded(entry);
    entry.paramsRow.hidden = filterHidden || !isExpanded;
    entry.summary.hidden = filterHidden || isExpanded;
    if (!entry.summary.hidden) renderSummary(entry);
  }

  function clearFieldInvalid(field) {
    var control = fieldControl(field);
    if (!control) return;
    control.removeAttribute("aria-invalid");
    var errorId = control.getAttribute("aria-describedby");
    var errorEl = errorId ? document.getElementById(errorId) : null;
    if (errorEl) errorEl.hidden = true;
  }

  function paramEntryFor(row) {
    for (var i = 0; i < paramEntries.length; i++) {
      if (paramEntries[i].row === row) return paramEntries[i];
    }
    return null;
  }

  // A row the server opened after a resubmission's field error starts
  // expanded regardless of the fresh-open default — set here, ahead of the
  // very first `sync()` pass (`applyFilter()`, below the guard).
  paramEntries.forEach(function (entry) {
    if (entry.paramsRow.hasAttribute("data-open")) {
      entry.toggle.setAttribute("aria-expanded", "true");
    }
  });

  paramEntries.forEach(function (entry) {
    entry.toggle.addEventListener("click", function () {
      entry.toggle.setAttribute("aria-expanded", expanded(entry) ? "false" : "true");
      sync(entry);
    });
    entry.fields.forEach(function (field) {
      var control = fieldControl(field);
      if (!control) return;
      ["input", "change"].forEach(function (eventName) {
        control.addEventListener(eventName, function () {
          clearFieldInvalid(field);
          if (!entry.summary.hidden) renderSummary(entry);
          updateCount();
        });
      });
    });
  });

  /* Blocks the submit on the first checked query still missing a required
     value — opens its row, marks and focuses the field — so an avoidable
     round trip to the server never happens. Returns whether the submit was
     blocked. */
  function blockSubmitOnMissingParams() {
    for (var i = 0; i < paramEntries.length; i++) {
      var entry = paramEntries[i];
      if (!entry.box || !entry.box.checked) continue;
      var missingField = entry.fields.filter(fieldIsMissing)[0];
      if (!missingField) continue;
      entry.toggle.setAttribute("aria-expanded", "true");
      sync(entry);
      var control = fieldControl(missingField);
      if (control) {
        control.setAttribute("aria-invalid", "true");
        var errorId = control.getAttribute("aria-describedby");
        var errorEl = errorId ? document.getElementById(errorId) : null;
        if (errorEl) {
          if (paramRequiredMessage) errorEl.textContent = paramRequiredMessage;
          errorEl.hidden = false;
        }
        control.focus();
      }
      return true;
    }
    return false;
  }

  // The since-custom check runs first (unchanged from #836) and, on
  // failure, focuses that field and stops — only when it passes does a
  // missing required parameter get its own turn at blocking the submit.
  form.addEventListener("submit", function (event) {
    sinceValidationStarted = true;
    if (!validateSinceCustom()) {
      event.preventDefault();
      sinceCustom.focus();
      return;
    }
    if (blockSubmitOnMissingParams()) {
      event.preventDefault();
    }
  });

  if (!table || !rows.length) return;

  var activeType = "all";

  function matchesType(row) {
    return activeType === "all" || row.dataset.kind === activeType;
  }

  function matchesFilter(row, needle) {
    return !needle || row.dataset.name.trim().toLowerCase().indexOf(needle) !== -1;
  }

  /* One linear pass over the rows per keystroke/click — no per-row reflow,
     no work that scales worse than the row count. Also the single place
     that re-syncs every parameterized row's visibility against the filter:
     a row's own `hidden` is set first, so `sync()` always reads this
     pass's fresh value. */
  function applyFilter() {
    var needle = filterInput ? filterInput.value.trim().toLowerCase() : "";
    var visible = 0;
    rows.forEach(function (row) {
      var match = matchesType(row) && matchesFilter(row, needle);
      row.hidden = !match;
      if (match) visible += 1;
    });
    if (emptyRow) emptyRow.hidden = visible !== 0;
    paramEntries.forEach(sync);
    updateSelectAll();
  }

  /* Reads live DOM state rather than re-deriving from the server-rendered
     `checked` attributes, so a browser-restored form (bfcache, back/
     forward) that came back with different boxes checked than the page's
     initial render still reports the count that is actually about to
     submit. Also appends "· k value(s) missing" whenever a checked query
     has an empty, default-less field. */
  function updateCount() {
    if (!countHint) return;
    var selected = rows.reduce(function (total, row) {
      var box = rowCheckbox(row);
      return total + (box && box.checked ? 1 : 0);
    }, 0);
    var text = countHint.dataset.msgCount
      .replace("{selected}", String(selected))
      .replace("{total}", String(rows.length));
    var missing = totalMissingCount();
    if (missing > 0) {
      var missingText = missingCountText(missing);
      if (missingText) text += " · " + missingText;
    }
    countHint.textContent = text;
  }

  /* The header checkbox mirrors only the rows the filter currently shows:
     checked when every visible row is checked, indeterminate when some are,
     unchecked (and disabled, nothing to act on) when the filter hides
     everything. */
  function updateSelectAll() {
    if (!selectAll) return;
    var visibleBoxes = rows
      .filter(function (row) {
        return !row.hidden;
      })
      .map(rowCheckbox)
      .filter(Boolean);
    if (!visibleBoxes.length) {
      selectAll.checked = false;
      selectAll.indeterminate = false;
      selectAll.disabled = true;
      return;
    }
    selectAll.disabled = false;
    var checkedCount = visibleBoxes.filter(function (box) {
      return box.checked;
    }).length;
    selectAll.checked = checkedCount === visibleBoxes.length;
    selectAll.indeterminate = checkedCount > 0 && checkedCount < visibleBoxes.length;
  }

  typeButtons.forEach(function (button) {
    button.addEventListener("click", function () {
      activeType = button.dataset.subjectFilter;
      typeButtons.forEach(function (candidate) {
        candidate.setAttribute("aria-pressed", candidate === button ? "true" : "false");
      });
      applyFilter();
    });
  });

  if (filterInput) {
    filterInput.addEventListener("input", applyFilter);
  }

  if (selectAll) {
    selectAll.addEventListener("change", function () {
      rows.forEach(function (row) {
        if (row.hidden) return;
        var box = rowCheckbox(row);
        if (box) box.checked = selectAll.checked;
        // #837: the select-all always opens a parameterized row it just
        // checked, the same "marking shows it open" rule a single checkbox
        // follows — the one it just unchecked needs no such push, `sync()`
        // already hides that one outright.
        var entry = paramEntryFor(row);
        if (entry) {
          if (selectAll.checked) entry.toggle.setAttribute("aria-expanded", "true");
          sync(entry);
        }
      });
      updateCount();
      updateSelectAll();
    });
  }

  rows.forEach(function (row) {
    var box = rowCheckbox(row);
    if (!box) return;
    box.addEventListener("change", function () {
      var entry = paramEntryFor(row);
      if (entry) {
        // (Re)checking a query always shows its values row open — there is
        // no prior fold state worth restoring on a first check.
        if (box.checked) entry.toggle.setAttribute("aria-expanded", "true");
        sync(entry);
      }
      updateCount();
      updateSelectAll();
    });
  });

  // Nothing above can run usefully without JavaScript, which is exactly why
  // the tools and the header checkbox render `hidden` on the server side —
  // reveal them only now that there is a script behind them.
  if (tools) tools.hidden = false;
  if (selectAll) selectAll.hidden = false;

  applyFilter();
  updateCount();

  // Focus the first field a resubmission flagged invalid, once its row has
  // been opened by the `data-open` pass above and the initial
  // `applyFilter()` sync — ahead of anything else competing for focus on
  // load.
  var invalidField = table.querySelector(".row--params[data-open] [aria-invalid='true']");
  if (invalidField) invalidField.focus();
})();
