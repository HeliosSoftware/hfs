/* Shared, progressively enhanced multi-select combobox. Transport belongs to
   htmx; this module owns only generic selection, keyboard and ARIA behavior.

   Paging (#1755): a result fragment may end with an empty
   `[data-combobox-more][data-page]` sentinel and a `[data-combobox-footer]`
   line. When the list is scrolled near its end, or ArrowDown is pressed on
   the last option, the sentinel is consumed and its token is posted back as
   `page`; the response is appended (never replacing the list), deduplicated by
   option value, and dropped if the query changed meanwhile. Sentinel and
   footer carry no `data-combobox-option`, so they are never options. */
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

  // #1755: whether a scrollable list is within `threshold` pixels of its end.
  // A list that does not overflow is always "at the bottom". Pure, so a Node
  // unit test can cover it without a DOM.
  function nearBottom(scrollTop, clientHeight, scrollHeight, threshold) {
    return scrollTop + clientHeight >= scrollHeight - threshold;
  }

  function parseValues(value) {
    var seen = Object.create(null);
    return String(value || "")
      .split(/[\r\n,]+/)
      .map(function (item) { return item.trim(); })
      .filter(function (item) {
        if (!item || seen[item]) return false;
        seen[item] = true;
        return true;
      });
  }

  // Same logical-ID grammar as the export endpoints. Existence remains a
  // server concern: ID-only backends may have no suggestions for a valid ID.
  function validReference(value, resourceType) {
    var prefix = resourceType + "/";
    var id = value.indexOf(prefix) === 0 ? value.slice(prefix.length) : value;
    return /^[A-Za-z0-9.-]{1,64}$/.test(id);
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
    var resourceType = root.getAttribute("data-combobox-resource-type");
    var validation = root.querySelector("[data-combobox-validation]");
    var values = [];
    var activeIndex = -1;
    var wantsOpen = false;
    // #1755: a next-page request is in flight; `loadQuery` is the input text
    // it was issued for, `pageToken` its `page` parameter, `pageBefore` the
    // listbox children that existed before it and `pageAdvance` whether the
    // keyboard asked for it (activate the first new option on arrival).
    var loadingMore = false;
    var loadQuery = "";
    var pageToken = "";
    var pageBefore = [];
    var pageAdvance = false;

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
      updateVisibility();
      if (content) status.textContent = content.textContent.trim();
      else {
        message("results", window.HfsNumber.format(options().length));
        var footer = listbox.querySelector("[data-combobox-footer]");
        if (footer && footer.textContent.trim()) status.textContent += ". " + footer.textContent.trim();
      }
    }

    function moreSentinel() {
      return listbox.querySelector("[data-combobox-more]");
    }

    // #1755: consume the sentinel and request the next page with the token it
    // carried, appending the response after the current options. The request
    // is sourced from the input, so `hx-sync="this:replace"` aborts it when
    // the user types a new query.
    function loadMore(opts) {
      var sentinel = moreSentinel();
      if (!sentinel || loadingMore || !window.htmx) return;
      pageToken = sentinel.getAttribute("data-page") || "";
      sentinel.remove();
      loadingMore = true;
      loadQuery = input.value;
      pageBefore = Array.prototype.slice.call(listbox.children);
      pageAdvance = Boolean(opts && opts.advance);
      root.setAttribute("aria-busy", "true");
      window.htmx.ajax("POST", input.getAttribute("hx-post"), {
        source: input,
        target: listbox,
        swap: "beforeend",
        values: { page: pageToken },
      });
    }

    function finishPage() {
      loadingMore = false;
      pageBefore = [];
      root.removeAttribute("aria-busy");
    }

    // #1755: reconcile an appended page with the list it extends.
    function absorbPage() {
      var added = Array.prototype.slice.call(listbox.children).filter(function (node) {
        return pageBefore.indexOf(node) < 0;
      });
      var advance = pageAdvance;
      var previous = pageBefore.filter(function (node) {
        return node.hasAttribute && node.hasAttribute("data-combobox-option");
      });
      var previousCount = previous.length;
      if (input.value !== loadQuery) {
        added.forEach(function (node) { node.remove(); });
        finishPage();
        return;
      }
      var seen = Object.create(null);
      previous.forEach(function (node) { seen[optionValue(node)] = true; });
      var addedOptions = 0;
      added.forEach(function (node) {
        if (!node.hasAttribute || !node.hasAttribute("data-combobox-option")) return;
        var value = optionValue(node);
        if (seen[value]) node.remove();
        else {
          seen[value] = true;
          addedOptions += 1;
        }
      });
      var footers = listbox.querySelectorAll("[data-combobox-footer]");
      var footer = footers.length ? footers[footers.length - 1] : null;
      Array.prototype.forEach.call(footers, function (node) {
        if (node !== footer) node.remove();
      });
      if (footer) listbox.appendChild(footer);
      synchronizeOptions();
      if (advance && previousCount < options().length) setActive(previousCount);
      updateVisibility();
      finishPage();
      message("results", window.HfsNumber.format(options().length));
      if (addedOptions > 0 && !listbox.hidden && moreSentinel() && listbox.scrollHeight <= listbox.clientHeight) {
        loadMore();
      }
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
      // Opening intent survives an empty result set. Only user interaction
      // can restore it after a close; a late response merely updates options.
      wantsOpen = open;
      updateVisibility();
    }

    function updateVisibility() {
      var hasOptions = options().length > 0;
      var next = Boolean(wantsOpen && !input.disabled && root.ownerDocument.activeElement === input && hasOptions);
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

    function clearValidation() {
      input.setCustomValidity("");
      input.removeAttribute("aria-invalid");
      if (validation) {
        validation.textContent = "";
        validation.hidden = true;
      }
    }

    function validPending(pendingValues) {
      return !resourceType || pendingValues.every(function (value) {
        return validReference(value, resourceType);
      });
    }

    function rejectPending(pending) {
      // A multi-line paste must remain editable in a type=search input,
      // which would otherwise discard its line separators.
      input.value = pending.replace(/[\r\n]+/g, ", ");
      var error = root.getAttribute("data-combobox-invalid-message");
      input.setCustomValidity(error);
      input.setAttribute("aria-invalid", "true");
      if (validation) {
        validation.textContent = error;
        validation.hidden = false;
      }
      setOpen(false);
    }

    function restoreValues(raw) {
      clearValidation();
      values = [];
      input.value = "";
      var restored = parseValues(raw);
      // Server validation may re-render rejected fallback values. Keep the
      // whole list editable instead of turning invalid references into chips.
      if (!validPending(restored)) rejectPending(raw);
      else restored.forEach(function (value) { add(value, value, false); });
    }

    function commitPending(raw) {
      // Export filters accept logical IDs and typed references, including
      // those with no lookup result. The table picker remains option-only.
      if (max !== 0 || input.disabled) return false;
      var pending = typeof raw === "string" ? raw : input.value;
      if (!pending.trim()) return false;
      var pendingValues = parseValues(pending);
      if (!validPending(pendingValues)) {
        rejectPending(pending);
        return true;
      }
      clearValidation();
      pendingValues.forEach(function (value) { add(value, value, true); });
      input.value = "";
      setOpen(false);
      return true;
    }

    function select(option) {
      clearValidation();
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
      // The search that produced this selection is consumed, not another
      // pending reference to append when an export form is submitted.
      if (max === 0) input.value = "";
      // Export selections consume the query even when already selected.
      // Keep the single-value table picker's existing search/result behavior.
      setActive(-1);
      setOpen(max !== 0);
    }

    input.addEventListener("keydown", function (event) {
      if (event.isComposing) return;
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
      if (event.key === "Enter" && activeIndex < 0 && commitPending()) {
        event.preventDefault();
        return;
      }
      if (event.key === "ArrowDown" && items.length && activeIndex === items.length - 1 && (loadingMore || moreSentinel())) {
        // #1755: the last option with more results behind it asks for them
        // instead of wrapping to the first option.
        event.preventDefault();
        if (!loadingMore) loadMore({ advance: true });
        return;
      }
      if (!items.length) {
        if (event.key === "ArrowDown" || event.key === "ArrowUp") {
          event.preventDefault();
          setOpen(true);
        }
        return;
      }
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

    input.addEventListener("paste", function (event) {
      if (max !== 0 || input.disabled || !event.clipboardData) return;
      var text = event.clipboardData.getData("text/plain");
      // A single pasted value remains a search or pending reference. Lists
      // must be parsed before type=search strips their line breaks.
      if (!/[\r\n,]/.test(text)) return;
      var start = input.selectionStart;
      var end = input.selectionEnd;
      if (start === null) start = input.value.length;
      if (end === null) end = start;
      var pending = input.value.slice(0, start) + text + input.value.slice(end);
      event.preventDefault();
      commitPending(pending);
    });

    input.addEventListener("focus", function () { setOpen(true); });
    input.addEventListener("input", function () {
      clearValidation();
      setOpen(true);
    });
    root.addEventListener("focusout", function (event) {
      if (!root.contains(event.relatedTarget)) setOpen(false);
    });
    listbox.addEventListener("scroll", function () {
      if (loadingMore || !moreSentinel()) return;
      if (nearBottom(listbox.scrollTop, listbox.clientHeight, listbox.scrollHeight, 48)) loadMore();
    }, { passive: true });
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
      if (event.detail.elt !== input) return;
      event.detail.parameters[queryName] = input.value;
      // Any request other than our own page request is a fresh search that
      // replaces the list, so a page still pending is no longer wanted.
      if (loadingMore && event.detail.parameters.page !== pageToken) loadingMore = false;
    });
    root.addEventListener("htmx:afterSwap", function (event) {
      if (loadingMore) {
        // Out-of-band message swaps also bubble here; only the list swap
        // carries the appended page.
        if (event.target === listbox) absorbPage();
        return;
      }
      root.removeAttribute("aria-busy");
      synchronizeResponse();
    });
    root.addEventListener("htmx:afterRequest", function () { if (!loadingMore) root.removeAttribute("aria-busy"); });
    root.addEventListener("htmx:responseError", function () {
      finishPage();
      showRequestError();
    });
    root.addEventListener("htmx:sendError", function () {
      finishPage();
      showRequestError();
    });

    var form = root.closest("form");
    // Capture precedes Bulk Export's inline submit validation, which checks
    // these hidden selections before it performs the native submission.
    if (form && max === 0) form.addEventListener("submit", function (event) {
      if (input.disabled) return;
      commitPending();
      if (!input.validity.valid) {
        event.preventDefault();
        // Stop caller submit handlers too: Bulk Export prepares a native
        // submit after loading assets, which bypasses constraint validation.
        event.stopImmediatePropagation();
        input.focus();
        setOpen(false);
      }
    }, true);
    if (form) form.addEventListener("reset", function () {
      window.setTimeout(function () {
        restoreValues(fallbackInput.defaultValue);
        listbox.replaceChildren();
        responseMessage.replaceChildren();
        responseMessage.hidden = true;
        status.textContent = "";
        setOpen(false);
        render();
        emitChange();
      }, 0);
    });

    restoreValues(fallbackInput.value);
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

  return { install: install, parseValues: parseValues, atCapacity: atCapacity, validReference: validReference, nearBottom: nearBottom };
});
