/*
 * SQL Query / SQL View page script: the Parameters card's (#841) and the
 * Tables panel's (#842) own JavaScript-only enhancement over their
 * otherwise plain-HTML mutations. Both `sql_parameters_card.html`'s Add
 * parameter/Declare buttons and `sql_tables_card.html`'s Add table/Remove
 * buttons already work with no JavaScript at all — `formaction` plus a
 * plain `<form>` submit that re-renders the whole page; their own
 * `hx-post`/`hx-target`/`hx-swap="outerHTML"` intercept that same submit
 * instead, swapping in just the card. This file wires only what neither of
 * those two paths already does on their own:
 *
 * 1. Once htmx settles a fresh `#lib-params` or `#lib-tables` that carries
 *    `data-document` (the `document` endpoint's own successful mutation
 *    response), it hands that text to
 *    `window.HfsSqlLibraryDetails.host.setDoc()` — the Details JSON pane's
 *    own undo-tracked host (`sql-library-details.js`) — so the mutation
 *    lands as one Ctrl+Z-able transaction that also refreshes the guided
 *    form and, via `setDoc`'s own `input` dispatch, re-fires the live run.
 *    The attribute is then stripped so a later, unrelated swap of the same
 *    element never re-applies it.
 *
 * 2. #842: when the Add table combobox's own selection changes
 *    (`hfs:combobox-select`, `combobox.js`), the Alias field is filled in
 *    with the chosen artifact's own bare name — but only when it is empty
 *    or still holds the *previous* autofill, never overwriting text the
 *    visitor typed by hand.
 *
 * 3. #842/04: clicking an unknown table's own *Declare {name}* button
 *    (`data-declare-table`, `sql_tables_card.html`) opens the *Add table*
 *    `<details>` and fills the Alias field with that row's own name —
 *    never a submit, unlike a Parameters hint's one-click *Declare*: the
 *    target itself still needs picking from the combobox. Event
 *    delegation, since the button is re-rendered on every `#lib-tables`
 *    swap the same way the combobox listener below already assumes.
 *
 * Both `#lib-params` and `#lib-tables` are always swapped wholesale
 * (`hx-swap="outerHTML"`, on every path that can produce either — the
 * page's own load, `/run`'s own OOB companion, and the `document`
 * endpoint's own direct-target response), so `event.target` is already the
 * settled replacement, the same idiom `sql-editor.js`'s own `#run-notice`
 * listener uses. Registered once, for the page's whole lifetime.
 *
 * Without `window.HfsSqlLibraryDetails` (the vendored bundle, or
 * `editor-pair.js`, never mounted, or a browser CodeMirror itself failed to
 * load in) `setDoc` falls back to writing the JSON textarea directly and
 * firing its own `input` event, exactly the plain-textarea contract every
 * other host in this family keeps.
 */
(function () {
  "use strict";

  var DOCUMENT_CARD_IDS = ["lib-params", "lib-tables"];

  function applyDataDocument(target) {
    var text = target.getAttribute("data-document");
    if (text === null) return;
    target.removeAttribute("data-document");

    var details = window.HfsSqlLibraryDetails;
    if (details && details.host) {
      details.host.setDoc(text);
      return;
    }
    var textarea = document.querySelector('textarea[name="json"][form="lib-editor-form"]');
    if (!textarea) return;
    textarea.value = text;
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
  }

  document.addEventListener("htmx:afterSwap", function (event) {
    var target = event.target;
    if (!target || DOCUMENT_CARD_IDS.indexOf(target.id) === -1) return;
    applyDataDocument(target);
  });

  // #842: the Add table combobox's own alias autocomplete. `lastAutofill`
  // remembers what this script itself last wrote, so choosing a second
  // option after the first still overwrites the field (nothing of the
  // visitor's own was lost) while a value the visitor typed by hand is
  // always left alone.
  var lastAutofill = null;

  document.addEventListener("hfs:combobox-select", function (event) {
    var root = event.target;
    if (!root || root.id !== "lib-tables-add-table") return;
    var name = event.detail && event.detail.name;
    if (!name) return;
    var alias = document.querySelector('input[name="table_alias"][form="lib-editor-form"]');
    if (!alias) return;
    if (alias.value === "" || alias.value === lastAutofill) {
      alias.value = name;
      lastAutofill = name;
    }
  });

  // #842/04: an unknown table's own *Declare {name}* button — opens the
  // Add table panel and pre-fills its alias with that row's own name,
  // overwriting whatever the panel already held (a visitor declaring a
  // second unknown table after the first means to replace it, not merge
  // with it). Deliberately *not* recorded in `lastAutofill`: the combobox
  // listener above only overwrites an alias it last wrote itself, so
  // picking the target VD afterwards leaves this exact spelling alone —
  // the whole point of *Declare* is to match the SQL's own (mis)spelled
  // table name, not the artifact's own name the combobox would offer.
  document.addEventListener("click", function (event) {
    var button = event.target.closest && event.target.closest("[data-declare-table]");
    if (!button) return;
    var name = button.getAttribute("data-declare-table");
    if (!name) return;
    var tablesCard = document.getElementById("lib-tables");
    if (!tablesCard) return;
    var details = tablesCard.querySelector(".editor-add");
    var alias = tablesCard.querySelector('input[name="table_alias"]');
    if (details) details.open = true;
    if (alias) alias.value = name;
  });
})();
