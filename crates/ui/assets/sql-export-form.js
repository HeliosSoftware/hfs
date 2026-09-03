/* Progressive enhancement for the SQL Export builder's subjects table
   (#834): a type switch, a text filter, a header select-all, and the
   "n of m selected" count, layered over rows and checkboxes that already
   render plain and already submit without this script (the create form
   itself is #833's work, restyled in #834's own template change).

   The one rule every piece of this file honors: filtering never unchecks a
   row. A row the type switch or the text filter hides keeps whatever
   checked state it had — it still submits with the form — so searching for
   the next subject in a long list never costs the user a selection already
   made. The header select-all only ever acts on the rows currently visible;
   the count below the table always counts every checked box, visible or
   not. */
(function () {
  "use strict";

  var form = document.querySelector("form.bulk-export-form");
  if (!form) return;

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
  if (!table || !rows.length) return;

  var activeType = "all";

  function rowCheckbox(row) {
    return row.querySelector('input[name="subject"]');
  }

  function matchesType(row) {
    return activeType === "all" || row.dataset.kind === activeType;
  }

  function matchesFilter(row, needle) {
    return !needle || row.dataset.name.trim().toLowerCase().indexOf(needle) !== -1;
  }

  /* One linear pass over the rows per keystroke/click — no per-row reflow,
     no work that scales worse than the row count. */
  function applyFilter() {
    var needle = filterInput ? filterInput.value.trim().toLowerCase() : "";
    var visible = 0;
    rows.forEach(function (row) {
      var match = matchesType(row) && matchesFilter(row, needle);
      row.hidden = !match;
      if (match) visible += 1;
    });
    if (emptyRow) emptyRow.hidden = visible !== 0;
    updateSelectAll();
  }

  /* Reads live DOM state rather than re-deriving from the server-rendered
     `checked` attributes, so a browser-restored form (bfcache, back/
     forward) that came back with different boxes checked than the page's
     initial render still reports the count that is actually about to
     submit. */
  function updateCount() {
    if (!countHint) return;
    var selected = rows.reduce(function (total, row) {
      var box = rowCheckbox(row);
      return total + (box && box.checked ? 1 : 0);
    }, 0);
    countHint.textContent = countHint.dataset.msgCount
      .replace("{selected}", String(selected))
      .replace("{total}", String(rows.length));
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
      });
      updateCount();
      updateSelectAll();
    });
  }

  rows.forEach(function (row) {
    var box = rowCheckbox(row);
    if (box) box.addEventListener("change", function () {
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
})();
