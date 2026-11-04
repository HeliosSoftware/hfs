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

  // HTMX navigation can evaluate this asset again. Reuse the runtime so
  // live mount identity and document listeners survive that evaluation;
  // cloned DOM elements still receive their own mounts.
  var api = root && root.HfsCombobox || factory();
  if (root) root.HfsCombobox = api;
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

  // #1755: how much longer the page-load footer must stay visible so the
  // state can be seen at all (a local server answers in ~10 ms).
  var MIN_LOADING_MS = 400;
  function remainingLoadingMs(startedAt, now, minimum) {
    return Math.max(0, minimum - (now - startedAt));
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

  // Classify before deduplicating: even repeated IDs are an intentional list.
  // Keep the insertion cursor when a single search loses surrounding separators.
  function preparePaste(value, text, start, end) {
    if (start === null) start = value.length;
    if (end === null) end = start;
    var before = value.slice(0, start);
    var after = value.slice(end);
    if (!parseValues(text).length) {
      return { count: 0, value: before + after, caret: start };
    }
    var pending = before + text + after;
    var entries = pending.split(/[\r\n,]+/).map(function (item) {
      return item.trim();
    }).filter(function (item) { return Boolean(item); });
    if (entries.length > 1) return { count: entries.length, value: pending };
    var normalized = entries[0];
    var caret = start + text.length - pending.indexOf(normalized);
    return { count: 1, value: normalized, caret: Math.max(0, Math.min(caret, normalized.length)) };
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

  var fixedMounts = new WeakMap();
  var installedScopes = new WeakSet();
  var fixedCounter = 0;

  // Viewport coordinates, independent of the scrollable editor containing the
  // control. Keep the popup in its DOM owner (including aria-modal dialogs).
  function fixedPlacement(rect, height, viewportWidth, viewportHeight) {
    var margin = 8;
    var gap = 4;
    var width = Math.max(0, Math.min(Math.max(rect.width, 160), viewportWidth - margin * 2));
    var below = Math.max(0, viewportHeight - rect.bottom - gap - margin);
    var above = Math.max(0, rect.top - gap - margin);
    var wanted = Math.min(height, 280);
    var up = wanted > below && (wanted <= above || above > below);
    var maxHeight = Math.min(280, up ? above : below);
    var actualHeight = Math.min(wanted, maxHeight);
    return {
      left: Math.max(margin, Math.min(rect.left, viewportWidth - width - margin)),
      top: up ? rect.top - gap - actualHeight : rect.bottom + gap,
      width: width,
      maxHeight: maxHeight,
    };
  }

  function createFixedRoot(native) {
    var doc = native.ownerDocument;
    var root = doc.createElement("span");
    root.id = native.id ? native.id + "-combobox" : "fixed-combobox-" + ++fixedCounter;
    root.className = "combobox combobox--fixed combobox--inline";
    root.setAttribute("data-combobox", "");
    root.setAttribute("data-combobox-mode", "fixed");
    root.setAttribute("data-combobox-generated", "");
    var enhancement = doc.createElement("span");
    enhancement.className = "combobox__enhancement";
    enhancement.setAttribute("data-combobox-enhancement", "");
    enhancement.hidden = true;
    var control = doc.createElement("span");
    control.className = "combobox__control";
    var trigger = doc.createElement("button");
    trigger.type = "button";
    trigger.className = "field__input combobox__trigger";
    trigger.disabled = true;
    trigger.setAttribute("role", "combobox");
    trigger.setAttribute("aria-expanded", "false");
    trigger.setAttribute("aria-haspopup", "listbox");
    trigger.setAttribute("aria-controls", root.id + "-listbox");
    var label = doc.createElement("span");
    label.id = root.id + "-value";
    label.setAttribute("data-combobox-value", "");
    var chevron = doc.createElement("span");
    chevron.className = "combobox__chevron";
    chevron.setAttribute("aria-hidden", "true");
    var svg = doc.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("viewBox", "0 0 12 12");
    var path = doc.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", "M2 4l4 4 4-4");
    path.setAttribute("fill", "none");
    path.setAttribute("stroke", "currentColor");
    path.setAttribute("stroke-width", "1.5");
    svg.appendChild(path);
    chevron.appendChild(svg);
    trigger.appendChild(label);
    trigger.appendChild(chevron);
    var listbox = doc.createElement("span");
    listbox.id = root.id + "-listbox";
    listbox.className = "combobox__listbox";
    listbox.setAttribute("data-combobox-listbox", "");
    listbox.setAttribute("role", "listbox");
    listbox.hidden = true;
    control.appendChild(trigger);
    control.appendChild(listbox);
    enhancement.appendChild(control);
    root.appendChild(enhancement);
    try {
      native.parentNode.insertBefore(root, native);
      root.insertBefore(native, enhancement);
    } catch (error) {
      if (root.parentNode) root.replaceWith(native);
      throw error;
    }
    return root;
  }

  // Opt-in enhancement of an existing SELECT. It remains the only value/form
  // control; business attributes and classes never move onto the trigger.
  function adopt(native, config) {
    if (!native || native.tagName !== "SELECT" || native.multiple || !native.parentNode) return null;
    var mounted = fixedMounts.get(native);
    if (mounted) {
      if (config) mounted.refresh(config);
      return mounted;
    }
    var doc = native.ownerDocument;
    var view = doc.defaultView;
    var root = native.closest('[data-combobox-mode="fixed"]');
    if (root && root.querySelector("[data-combobox-native]") !== native) root = null;
    var available = config && typeof config.available === "boolean"
      ? config.available : root ? !root.hidden : !native.hidden;
    var originalNativeMarker = native.getAttribute("data-combobox-native");
    // Read real labels before inserting generated option text into the owner.
    var fallbackLabel = Array.prototype.map.call(native.labels || [], function (node) {
      var copy = node.cloneNode(true);
      copy.querySelectorAll("select, [data-combobox-enhancement]").forEach(function (child) { child.remove(); });
      return copy.textContent.trim();
    }).filter(Boolean).join(" ");
    try {
      if (!root) root = createFixedRoot(native);
    } catch (_) {
      if (root && root.hasAttribute("data-combobox-generated") && root.parentNode) root.replaceWith(native);
      return null;
    }
    var enhancement = root.querySelector("[data-combobox-enhancement]");
    var trigger = root.querySelector('[role="combobox"]');
    var label = root.querySelector("[data-combobox-value]");
    var listbox = root.querySelector("[data-combobox-listbox]");
    if (!enhancement || !trigger || !label || !listbox) {
      native.hidden = !available;
      root.removeAttribute("data-combobox-ready");
      if (enhancement) enhancement.hidden = true;
      if (trigger) trigger.disabled = true;
      if (root.hasAttribute("data-combobox-generated") && root.parentNode) root.replaceWith(native);
      return null;
    }
    var cleanup = [];
    var activeIndex = -1;
    var items = [];
    var signature = "";
    var destroyed = false;
    var resetTimer = 0;
    var anchorRect = null;
    // The pre-existing fixed_select macro's Since consumers depend on these
    // notifications. Newly adopted SELECTs retain native silent reset semantics.
    var notifyRestore = !root.hasAttribute("data-combobox-generated");
    function listen(target, event, handler) {
      target.addEventListener(event, handler);
      cleanup.push(function () { target.removeEventListener(event, handler); });
    }
    function optionDisabled(index) {
      var option = native.options[index];
      return !option || option.disabled || (option.parentElement.tagName === "OPTGROUP" && option.parentElement.disabled);
    }
    function firstEnabled(start, step) {
      for (var i = start; i >= 0 && i < native.options.length; i += step) {
        if (!optionDisabled(i)) return i;
      }
      return -1;
    }
    function disabled() {
      return native.disabled || native.matches(":disabled");
    }
    function setActive(index) {
      activeIndex = index;
      items.forEach(function (item, i) {
        item.classList.toggle("combobox__option--active", i === index);
      });
      if (index < 0 || !items[index]) trigger.removeAttribute("aria-activedescendant");
      else {
        var item = items[index];
        trigger.setAttribute("aria-activedescendant", item.id);
        // Scroll the list alone, never its editor tree or the page behind it.
        if (item.offsetTop < listbox.scrollTop) listbox.scrollTop = item.offsetTop;
        else if (item.offsetTop + item.offsetHeight > listbox.scrollTop + listbox.clientHeight) {
          listbox.scrollTop = item.offsetTop + item.offsetHeight - listbox.clientHeight;
        }
      }
    }
    function close(event) {
      if (event && event.type === "scroll") {
        if (listbox.hidden || listbox.contains(event.target)) return;
        var current = trigger.getBoundingClientRect();
        // A scroll queued by focus/auto-scroll can arrive after open() has
        // already measured the final anchor. Only dismiss if it actually moved.
        if (anchorRect && ["left", "top", "width", "height"].every(function (key) {
          return Math.abs(current[key] - anchorRect[key]) < 0.5;
        })) return;
      }
      anchorRect = null;
      listbox.hidden = true;
      trigger.setAttribute("aria-expanded", "false");
      setActive(-1);
    }
    function synchronize() {
      var option = native.options[native.selectedIndex];
      label.textContent = option ? option.textContent : "";
      var labelledBy = native.getAttribute("aria-labelledby");
      var name = native.getAttribute("aria-label") || fallbackLabel || native.title || native.name;
      if (labelledBy) {
        trigger.setAttribute("aria-labelledby", labelledBy + " " + label.id);
        trigger.removeAttribute("aria-label");
        listbox.setAttribute("aria-labelledby", labelledBy);
        listbox.removeAttribute("aria-label");
      } else {
        trigger.removeAttribute("aria-labelledby");
        trigger.setAttribute("aria-label", (name + " " + label.textContent).trim());
        listbox.removeAttribute("aria-labelledby");
        listbox.setAttribute("aria-label", name);
      }
      ["aria-describedby", "aria-invalid"].forEach(function (attr) {
        if (native.hasAttribute(attr)) trigger.setAttribute(attr, native.getAttribute(attr));
        else trigger.removeAttribute(attr);
      });
      trigger.disabled = disabled() || firstEnabled(0, 1) < 0;
      items.forEach(function (item, index) {
        item.setAttribute("aria-selected", String(index === native.selectedIndex));
      });
    }
    function refresh(options) {
      if (destroyed) return null;
      if (options && typeof options.available === "boolean") available = options.available;
      close();
      var nextSignature = JSON.stringify(Array.prototype.map.call(native.options, function (option, index) {
        return [option.value, option.textContent, Boolean(optionDisabled(index))];
      }));
      if (signature !== nextSignature) {
        listbox.replaceChildren();
        items = [];
        Array.prototype.forEach.call(native.options, function (option, index) {
          var item = doc.createElement("span");
          item.className = "combobox__option";
          item.id = root.id + "-option-" + index;
          item.setAttribute("role", "option");
          item.setAttribute("data-value", option.value);
          item.setAttribute("aria-disabled", String(Boolean(optionDisabled(index))));
          item.textContent = option.textContent;
          listbox.appendChild(item);
          items.push(item);
        });
        signature = nextSignature;
      }
      synchronize();
      root.hidden = !available;
      native.hidden = true;
      return api;
    }
    function position() {
      var rect = trigger.getBoundingClientRect();
      anchorRect = rect;
      var width = view.innerWidth || doc.documentElement.clientWidth;
      var height = view.innerHeight || doc.documentElement.clientHeight;
      var placement = fixedPlacement(rect, 0, width, height);
      listbox.style.width = placement.width + "px";
      listbox.style.maxHeight = "280px";
      placement = fixedPlacement(rect, listbox.scrollHeight + 2, width, height);
      listbox.style.left = placement.left + "px";
      listbox.style.top = placement.top + "px";
      listbox.style.maxHeight = placement.maxHeight + "px";
    }
    function open() {
      refresh();
      if (!available || trigger.disabled || !native.isConnected) return;
      listbox.hidden = false;
      trigger.setAttribute("aria-expanded", "true");
      position();
      setActive(optionDisabled(native.selectedIndex) ? firstEnabled(0, 1) : native.selectedIndex);
    }
    function choose(index) {
      // The owner can lock a projection or replace its options while this
      // popup is open. Recheck the source, not only the last painted button.
      if (destroyed || !available || disabled() || !native.isConnected || optionDisabled(index) ||
          !items[index] || items[index].getAttribute("data-value") !== native.options[index].value) {
        refresh();
        return;
      }
      native.selectedIndex = index;
      synchronize();
      close();
      trigger.focus();
      native.dispatchEvent(new view.Event("input", { bubbles: true }));
      native.dispatchEvent(new view.Event("change", { bubbles: true }));
    }
    function destroy() {
      if (destroyed) return;
      destroyed = true;
      view.clearTimeout(resetTimer);
      cleanup.reverse().forEach(function (remove) { remove(); });
      close();
      listbox.replaceChildren();
      trigger.disabled = true;
      enhancement.hidden = true;
      native.hidden = !available;
      root.hidden = !available;
      root.removeAttribute("data-combobox-ready");
      if (originalNativeMarker === null) native.removeAttribute("data-combobox-native");
      else native.setAttribute("data-combobox-native", originalNativeMarker);
      fixedMounts.delete(native);
      if (root.hasAttribute("data-combobox-generated") && root.parentNode) root.replaceWith(native);
    }
    var api = { root: root, trigger: trigger, refresh: refresh, close: close, destroy: destroy,
      focus: function () { if (!destroyed && available) trigger.focus(); } };
    try {
      native.setAttribute("data-combobox-native", "");
      trigger.setAttribute("data-combobox-trigger", "");
      listen(trigger, "click", function () { if (listbox.hidden) open(); else close(); });
      listen(trigger, "keydown", function (event) {
        var key = event.key;
        if (key === "Tab" || key === "Escape") {
          if (key === "Escape" && !listbox.hidden) {
            event.preventDefault();
            event.stopPropagation();
          }
          close();
        } else if (["ArrowDown", "ArrowUp", "Home", "End", "Enter", " "].indexOf(key) >= 0) {
          event.preventDefault();
          var wasClosed = listbox.hidden;
          if (wasClosed) open();
          if (listbox.hidden) return;
          if (key === "Home") setActive(firstEnabled(0, 1));
          else if (key === "End") setActive(firstEnabled(items.length - 1, -1));
          else if ((key === "Enter" || key === " ") && !wasClosed) choose(activeIndex);
          else if (!wasClosed && key === "ArrowDown") {
            var next = firstEnabled(activeIndex + 1, 1);
            if (next >= 0) setActive(next);
          } else if (!wasClosed && key === "ArrowUp") {
            var previous = firstEnabled(activeIndex - 1, -1);
            if (previous >= 0) setActive(previous);
          }
        }
      });
      listen(listbox, "mousedown", function (event) { event.preventDefault(); });
      listen(listbox, "click", function (event) {
        var option = event.target.closest('[role="option"]');
        var index = items.indexOf(option);
        if (index >= 0) choose(index);
      });
      listen(root, "focusout", function (event) { if (!root.contains(event.relatedTarget)) close(); });
      listen(root, "hfs:combobox-close", close);
      listen(native, "change", function () { refresh(); });
      if (native.form) listen(native.form, "reset", function () {
        // Reset fires before the browser restores default selectedness.
        view.clearTimeout(resetTimer);
        resetTimer = view.setTimeout(function () {
          if (destroyed || !native.isConnected) return;
          refresh();
          if (notifyRestore) native.dispatchEvent(new view.Event("change", { bubbles: true }));
        }, 0);
      });
      listen(root, "hfs:combobox-restore", function () {
        if (!native.isConnected) return;
        refresh();
        if (notifyRestore) native.dispatchEvent(new view.Event("change", { bubbles: true }));
      });
      refresh();
      enhancement.hidden = false;
      root.setAttribute("data-combobox-ready", "true");
      fixedMounts.set(native, api);
      return api;
    } catch (_) {
      destroy();
      return null;
    }
  }

  function refreshFixed(native, config) {
    var mounted = fixedMounts.get(native);
    return mounted ? mounted.refresh(config) : adopt(native, config);
  }

  function destroyFixed(scope) {
    if (!scope) return;
    var natives = scope.matches && scope.matches("select") ? [scope]
      : Array.prototype.slice.call(scope.querySelectorAll("select[data-combobox-native]"));
    natives.forEach(function (native) {
      var mounted = fixedMounts.get(native);
      if (mounted) mounted.destroy();
    });
  }

  function initialize(root) {
    if (root.getAttribute("data-combobox-mode") === "fixed") {
      return adopt(root.querySelector("[data-combobox-native]"));
    }
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
    // #1755: the footer currently showing the loading state, its normal text,
    // the localized loading label, when loading began and the restore timer.
    var loadingFooter = null;
    var footerText = "";
    var loadingLabel = "";
    var loadingStarted = 0;
    var footerTimer = 0;

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
      showFooterLoading(listbox.querySelector("[data-combobox-footer]"));
      window.htmx.ajax("POST", input.getAttribute("hx-post"), {
        source: input,
        target: listbox,
        swap: "beforeend",
        values: { page: pageToken },
      });
    }

    function cancelFooterTimer() {
      if (footerTimer) window.clearTimeout(footerTimer);
      footerTimer = 0;
    }

    // Put the footer back to its normal text and drop the loading state.
    function restoreFooter() {
      cancelFooterTimer();
      var footer = loadingFooter;
      loadingFooter = null;
      if (!footer || !footer.isConnected) return;
      footer.classList.remove("combobox__footer--loading");
      footer.textContent = footerText;
    }

    // Show a spinner and the loading label in place of the footer text.
    function showFooterLoading(footer) {
      if (!footer) return;
      cancelFooterTimer();
      if (loadingFooter !== footer) {
        if (loadingFooter) restoreFooter();
        footerText = footer.textContent;
        loadingFooter = footer;
      }
      loadingLabel = footer.getAttribute("data-loading-label") || loadingLabel;
      loadingStarted = Date.now();
      var doc = root.ownerDocument;
      var spinner = doc.createElement("span");
      spinner.className = "spinner";
      spinner.setAttribute("aria-hidden", "true");
      footer.classList.add("combobox__footer--loading");
      footer.replaceChildren(spinner);
      if (loadingLabel) {
        var label = doc.createElement("span");
        label.textContent = loadingLabel;
        footer.appendChild(label);
      }
    }

    // Keep the loading footer up for the minimum time, then restore it.
    function restoreFooterSoon() {
      cancelFooterTimer();
      var wait = remainingLoadingMs(loadingStarted, Date.now(), MIN_LOADING_MS);
      if (wait <= 0) restoreFooter();
      else footerTimer = window.setTimeout(restoreFooter, wait);
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
        restoreFooter();
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
      if (footer && loadingFooter) {
        // The response brought a fresh footer; hold it in the loading state
        // until the minimum duration has passed.
        if (footer !== loadingFooter) {
          var started = loadingStarted;
          loadingFooter = null;
          showFooterLoading(footer);
          loadingStarted = started;
        }
        restoreFooterSoon();
      } else {
        restoreFooter();
      }
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
        // A load already in flight (e.g. started by the scroll that End
        // caused) must not swallow the key: ask it to advance when it lands.
        if (loadingMore) pageAdvance = true;
        else loadMore({ advance: true });
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
      var pasted = preparePaste(input.value, text, input.selectionStart, input.selectionEnd);
      event.preventDefault();
      if (pasted.count > 1) commitPending(pasted.value);
      else {
        input.value = pasted.value;
        input.setSelectionRange(pasted.caret, pasted.caret);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      }
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
      if (event.detail.parameters.page === undefined) restoreFooter();
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
      restoreFooter();
      finishPage();
      showRequestError();
    });
    root.addEventListener("htmx:sendError", function () {
      restoreFooter();
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
    if (scope.matches && scope.matches("[data-combobox]")) initialize(scope);
    Array.prototype.slice.call(scope.querySelectorAll("[data-combobox]")).forEach(initialize);
    if (scope.matches && scope.matches("select[data-combobox-select]")) adopt(scope);
    Array.prototype.slice.call(scope.querySelectorAll("select[data-combobox-select]")).forEach(function (native) {
      adopt(native);
    });
    if (installedScopes.has(scope)) return;
    installedScopes.add(scope);
    // Delegate restoration through live elements so a window listener does
    // not retain every fixed field removed by an HTMX navigation.
    if (scope.nodeType === 9) {
      scope.defaultView.addEventListener("pageshow", function () {
        scope.querySelectorAll('[data-combobox-mode="fixed"][data-combobox-ready="true"]').forEach(function (root) {
          root.dispatchEvent(new scope.defaultView.Event("hfs:combobox-restore"));
        });
      });
      // Delegate through live DOM so removed projections cannot be retained by
      // per-control window listeners. Ignore queued scrolls whose anchor has
      // already settled, as well as scrolling the option list itself.
      function closeFixedPopups(event) {
        scope.querySelectorAll('[data-combobox-mode="fixed"][data-combobox-ready="true"]').forEach(function (root) {
          var mounted = fixedMounts.get(root.querySelector("[data-combobox-native]"));
          if (mounted) mounted.close(event);
        });
      }
      scope.defaultView.addEventListener("scroll", closeFixedPopups, true);
      scope.defaultView.addEventListener("resize", closeFixedPopups);
      scope.addEventListener("htmx:beforeCleanupElement", function (event) { destroyFixed(event.target); });
    }
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

  return { install: install, adopt: adopt, refresh: refreshFixed, destroy: destroyFixed,
    fixedPlacement: fixedPlacement, parseValues: parseValues, preparePaste: preparePaste,
    atCapacity: atCapacity, validReference: validReference, nearBottom: nearBottom, remainingLoadingMs: remainingLoadingMs };
});
