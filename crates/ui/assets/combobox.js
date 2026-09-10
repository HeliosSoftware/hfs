/* Shared, progressively enhanced multi-select combobox. Transport belongs to
   htmx; this module owns only generic selection, keyboard and ARIA behavior. */
(function (root, factory) {
  "use strict";

  var api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root && root.document) api.install(root.document);
})(typeof window !== "undefined" ? window : null, function () {
  "use strict";

  // #842: whether choosing one more option, on top of `currentCount`
  // already selected, must first clear the field instead of adding —
  // single-value mode (`data-combobox-max="1"`) once it already holds one.
  // `max <= 0` (every caller before #842's own *Add table* field) never
  // hits capacity, matching this field's pre-#842 unlimited behavior
  // exactly. Exported (and used by `add()` below, never duplicated) so a
  // Node unit test can exercise the single-value decision without a DOM.
  function atCapacity(currentCount, max) {
    return max > 0 && currentCount >= max;
  }

  function parseValues(value) {
    var seen = Object.create(null);
    return String(value || "")
      .split(/[\n,]+/)
      .map(function (item) { return item.trim(); })
      .filter(function (item) {
        if (!item || seen[item]) return false;
        seen[item] = true;
        return true;
      });
  }

  function optionValue(option) {
    return option.getAttribute("data-value") || option.getAttribute("value") || "";
  }

  function optionLabel(option) {
    return option.getAttribute("data-label") || option.textContent.trim();
  }

  // #842: only `table_options` (`lookup_options.html`) ever sets
  // `data-name` — the bare artifact name, distinct from `optionLabel`'s own
  // " — ViewDefinition"/" — SQL View" suffix (see `LookupOption::name`'s own
  // doc comment, `crates/ui/src/lookup.rs`). Every other option (Patient,
  // Group) carries none, so this is `""` for them.
  function optionName(option) {
    return option.getAttribute("data-name") || "";
  }

  function initialize(root) {
    if (root.getAttribute("data-combobox-ready") === "true") return;

    var name = root.getAttribute("data-combobox-name");
    var queryName = root.getAttribute("data-combobox-query-name");
    var fallback = root.querySelector("[data-combobox-fallback]");
    var fallbackInput = fallback && fallback.querySelector('[name="' + name + '"]');
    var enhancement = root.querySelector("[data-combobox-enhancement]");
    var input = root.querySelector('[role="combobox"]');
    var listbox = root.querySelector("[data-combobox-listbox]");
    var responseMessage = root.querySelector("[data-combobox-message]");
    var selections = root.querySelector("[data-combobox-selections]");
    var status = root.querySelector("[data-combobox-status]");
    if (!name || !queryName || !fallback || !fallbackInput || !enhancement || !input || !listbox || !responseMessage || !selections || !status) return;

    // #842: `data-combobox-max="1"` (the only value any caller sets today)
    // switches this instance to single-value mode — choosing an option
    // replaces the current selection rather than adding to it, so `values`
    // never holds more than one entry. `0` (the default, every other
    // caller) means unlimited, matching this field's pre-#842 behavior
    // exactly.
    var max = parseInt(root.getAttribute("data-combobox-max"), 10) || 0;
    // #842: `data-combobox-form` — set only by a caller whose fieldset sits
    // outside the `<form>` it submits with (`sql_tables_card.html`'s own
    // "form-associated" fields, `form="lib-editor-form"` on each of its
    // siblings). Every hidden input `render()` creates below needs the same
    // `form` attribute the fallback textarea already carries, or neither a
    // plain submit nor `hx-include="#{form}"` (which serializes via the
    // browser's own `FormData(form)`, form-associated elements only) ever
    // sees a selection made through this field. Absent for every caller
    // whose fieldset already lives inside its `<form>` (Bulk Export, SQL
    // Export) — natural descendants need no explicit association.
    var formId = root.getAttribute("data-combobox-form") || "";
    var values = [];
    var activeIndex = -1;

    function message(key, detail) {
      var prefix = root.getAttribute("data-combobox-" + key + "-message") || "";
      status.textContent = detail ? prefix + " " + detail : prefix;
    }

    function applyAlternateCopy() {
      var hint = root.querySelector("[data-combobox-hint]");
      var alternateHint = root.getAttribute("data-combobox-alternate-hint");
      var alternatePlaceholder = root.getAttribute("data-combobox-alternate-placeholder");
      if (hint && alternateHint) hint.textContent = alternateHint;
      if (alternatePlaceholder) input.setAttribute("placeholder", alternatePlaceholder);
      root.setAttribute("data-combobox-mode", "alternate");
    }

    function synchronizeResponse() {
      var content = responseMessage.querySelector("[data-combobox-message-content]");
      var useAlternate = responseMessage.querySelector("[data-combobox-use-alternate]");
      responseMessage.hidden = !content;
      if (useAlternate) applyAlternateCopy();
      synchronizeOptions();
      // A swap replaces the option nodes. Never retain an active index or
      // aria-activedescendant that referred to the previous result set.
      setActive(-1);
      setOpen(true);
      if (content) status.textContent = content.textContent.trim();
      else message("results", String(options().length));
    }

    function showRequestError() {
      var text = root.getAttribute("data-combobox-error-message") || "";
      responseMessage.replaceChildren();
      if (text) {
        var error = root.ownerDocument.createElement("span");
        error.className = "field__hint field__hint--error";
        error.setAttribute("data-combobox-message-content", "");
        error.textContent = text;
        responseMessage.appendChild(error);
      }
      responseMessage.hidden = !text;
      status.textContent = text;
      setOpen(false);
    }

    function options() {
      return Array.prototype.slice.call(listbox.querySelectorAll("[data-combobox-option]"));
    }

    function setOpen(open) {
      var hasOptions = options().length > 0;
      var next = Boolean(open && hasOptions);
      input.setAttribute("aria-expanded", String(next));
      listbox.hidden = !next;
      if (!next) setActive(-1);
    }

    function setActive(index) {
      var items = options();
      items.forEach(function (option, itemIndex) {
        option.classList.toggle("combobox__option--active", itemIndex === index);
      });
      activeIndex = index >= 0 && index < items.length ? index : -1;
      if (activeIndex < 0) input.removeAttribute("aria-activedescendant");
      else {
        var active = items[activeIndex];
        if (!active.id) active.id = root.id + "-option-" + activeIndex;
        input.setAttribute("aria-activedescendant", active.id);
        active.scrollIntoView({ block: "nearest" });
      }
    }

    function synchronizeOptions() {
      options().forEach(function (option, index) {
        if (!option.id) option.id = root.id + "-option-" + index;
        option.setAttribute("role", "option");
        option.setAttribute("aria-selected", String(values.some(function (item) {
          return item.value === optionValue(option);
        })));
      });
    }

    function emitChange() {
      root.dispatchEvent(new CustomEvent("hfs:combobox-change", {
        bubbles: true,
        detail: { values: values.slice() },
      }));
    }

    function render() {
      var ownerDocument = root.ownerDocument;
      while (selections.firstChild) selections.removeChild(selections.firstChild);
      root.querySelectorAll("[data-combobox-selected-input]").forEach(function (selectedInput) {
        selectedInput.remove();
      });

      values.forEach(function (item) {
        var hidden = ownerDocument.createElement("input");
        hidden.type = "hidden";
        hidden.name = name;
        hidden.value = item.value;
        hidden.setAttribute("data-combobox-selected-input", "");
        if (formId) hidden.setAttribute("form", formId);
        root.appendChild(hidden);

        var chip = ownerDocument.createElement("span");
        chip.className = "chip combobox__chip";
        chip.setAttribute("role", "listitem");
        var text = ownerDocument.createElement("span");
        text.className = "combobox__chip-label";
        text.textContent = item.label;
        chip.appendChild(text);

        var remove = ownerDocument.createElement("button");
        remove.type = "button";
        remove.className = "combobox__remove";
        remove.setAttribute("data-combobox-remove", item.value);
        remove.setAttribute("aria-label", (root.getAttribute("data-combobox-remove-label") || "Remove") + " " + item.label);
        remove.textContent = "×";
        chip.appendChild(remove);
        selections.appendChild(chip);
      });
      selections.hidden = values.length === 0;
      synchronizeOptions();
    }

    function add(value, label, announce) {
      if (!value || values.some(function (item) { return item.value === value; })) return false;
      // #842: single-value mode — choosing an option replaces whatever was
      // already selected instead of adding a second chip.
      if (atCapacity(values.length, max)) values = [];
      values.push({ value: value, label: label || value });
      render();
      if (announce) message("added", label || value);
      emitChange();
      return true;
    }

    function remove(value) {
      var removed = values.find(function (item) { return item.value === value; });
      if (!removed) return;
      values = values.filter(function (item) { return item.value !== value; });
      render();
      message("removed", removed.label);
      emitChange();
      input.focus();
    }

    function select(option) {
      var value = optionValue(option);
      var label = optionLabel(option);
      var added = add(value, label, true);
      // #842: only on an actual selection (never a repeat click on the
      // option already chosen, which `add` no-ops) — the payload a caller
      // like `sql-library-panels.js` needs to autofill a sibling field from
      // the option's own bare name, distinct from `hfs:combobox-change`'s
      // whole-values-list payload every combobox already emits.
      if (added) {
        root.dispatchEvent(new CustomEvent("hfs:combobox-select", {
          bubbles: true,
          detail: { value: value, label: label, name: optionName(option) },
        }));
      }
      // Keep results open so another item can be chosen without repeating the
      // query. Escape, Tab and outside clicks remain the explicit close paths.
      setActive(-1);
      setOpen(true);
    }

    input.addEventListener("keydown", function (event) {
      var items = options();
      if (event.key === "Escape") {
        // Prevent type=search from clearing itself and emitting a new `search`
        // request that would reopen the list immediately.
        event.preventDefault();
        setOpen(false);
        return;
      }
      if (event.key === "Tab") {
        setOpen(false);
        return;
      }
      if (!items.length) return;
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setOpen(true);
        setActive(activeIndex < items.length - 1 ? activeIndex + 1 : 0);
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        setOpen(true);
        setActive(activeIndex > 0 ? activeIndex - 1 : items.length - 1);
      } else if (event.key === "Home") {
        event.preventDefault();
        setOpen(true);
        setActive(0);
      } else if (event.key === "End") {
        event.preventDefault();
        setOpen(true);
        setActive(items.length - 1);
      } else if (event.key === "Enter" && activeIndex >= 0) {
        event.preventDefault();
        select(items[activeIndex]);
      }
    });

    input.addEventListener("focus", function () { setOpen(true); });
    listbox.addEventListener("mousedown", function (event) { event.preventDefault(); });
    listbox.addEventListener("click", function (event) {
      var option = event.target.closest("[data-combobox-option]");
      if (option && listbox.contains(option)) select(option);
    });
    selections.addEventListener("click", function (event) {
      var button = event.target.closest("[data-combobox-remove]");
      if (button) remove(button.getAttribute("data-combobox-remove"));
    });
    root.addEventListener("hfs:combobox-close", function () { setOpen(false); });

    root.addEventListener("htmx:beforeRequest", function () {
      root.setAttribute("aria-busy", "true");
      message("loading");
    });
    root.addEventListener("htmx:configRequest", function (event) {
      if (event.detail.elt === input) event.detail.parameters[queryName] = input.value;
    });
    root.addEventListener("htmx:afterSwap", function () {
      root.removeAttribute("aria-busy");
      synchronizeResponse();
    });
    root.addEventListener("htmx:afterRequest", function () { root.removeAttribute("aria-busy"); });
    root.addEventListener("htmx:responseError", function () {
      root.removeAttribute("aria-busy");
      showRequestError();
    });
    root.addEventListener("htmx:sendError", function () {
      root.removeAttribute("aria-busy");
      showRequestError();
    });

    var form = root.closest("form");
    if (form) form.addEventListener("reset", function () {
      window.setTimeout(function () {
        values = [];
        parseValues(fallbackInput.defaultValue).forEach(function (value) { add(value, value, false); });
        input.value = "";
        listbox.replaceChildren();
        responseMessage.replaceChildren();
        responseMessage.hidden = true;
        status.textContent = "";
        setOpen(false);
        render();
        emitChange();
      }, 0);
    });

    parseValues(fallbackInput.value).forEach(function (value) { add(value, value, false); });
    render();
    fallbackInput.disabled = true;
    fallback.hidden = true;
    enhancement.hidden = false;
    input.disabled = false;
    root.setAttribute("data-combobox-ready", "true");
  }

  function install(scope) {
    Array.prototype.slice.call(scope.querySelectorAll("[data-combobox]")).forEach(initialize);
    scope.addEventListener("click", function (event) {
      scope.querySelectorAll('[data-combobox][data-combobox-ready="true"]').forEach(function (root) {
        if (!root.contains(event.target)) root.dispatchEvent(new CustomEvent("hfs:combobox-close"));
      });
    });
  }

  // #842/04: an htmx OOB swap (`sql_tables_card.html`'s own `#lib-tables`,
  // re-rendered whenever the unknown-table lint's own findings change the
  // Tables panel's signature) replaces a `[data-combobox]` field with a
  // fresh, un-enhanced one straight from the server — `initialize()` never
  // ran on it, so it would stay the plain fallback textarea, disabled
  // search input and all. `event.target` is the settled swapped-in element
  // itself (the same idiom `sql-editor.js`/`sql-library-panels.js` already
  // use for their own `htmx:afterSwap` listeners), so `install(target)`
  // only ever re-scans a freshly inserted subtree — never `document` as a
  // whole, which would re-register `install`'s own `click` listener on it
  // every single swap. `initialize()`'s own `data-combobox-ready` guard
  // makes this safe to call on content that already contains a live,
  // enhanced combobox (a swap elsewhere on the page that happens to be an
  // ancestor of one) — it simply does nothing for those.
  if (typeof document !== "undefined") {
    document.addEventListener("htmx:afterSwap", function (event) {
      var target = event.target;
      if (target && typeof target.querySelectorAll === "function") install(target);
    });
  }

  return { install: install, parseValues: parseValues, atCapacity: atCapacity };
});
