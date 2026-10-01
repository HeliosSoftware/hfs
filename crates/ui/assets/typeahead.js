/* Shared single-value typeahead for free-text inputs. Pure vanilla JS: it owns
   tolerant filtering, keyboard and ARIA combobox semantics, and never alters
   the typed text. Callers supply the catalogue through `attach`'s config. */
(function (root, factory) {
  "use strict";

  var api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.HfsTypeahead = api;
})(typeof window !== "undefined" ? window : null, function () {
  "use strict";

  var MIN_WIDTH = 260;
  var GAP = 4;
  var counter = 0;

  /* Lower-cases `text` and drops hyphens and underscores, so that
     "_lastUpdated" and "address-city" compare as "lastupdated" and
     "addresscity". */
  function normalize(text) {
    return String(text == null ? "" : text).toLowerCase().replace(/[-_]/g, "");
  }

  /* Filters `options` (`[{ value, hint }]`) by `query`. An empty query returns
     a copy in the original order. Otherwise three groups are concatenated, each
     keeping the original order: name starts with the query, name contains it,
     and finally hint starts with it. No option appears twice. */
  function filterOptions(options, query) {
    var list = options || [];
    var trimmed = String(query == null ? "" : query).trim();
    if (!trimmed) return list.slice();

    var needle = normalize(trimmed);
    var literal = trimmed.toLowerCase();
    var useLiteral = !needle;
    var prefix = [];
    var inner = [];
    var byHint = [];

    list.forEach(function (option) {
      var value = String(option.value == null ? "" : option.value);
      var hint = String(option.hint == null ? "" : option.hint).toLowerCase();
      var haystack = useLiteral ? value.toLowerCase() : normalize(value);
      var wanted = useLiteral ? literal : needle;
      var at = haystack.indexOf(wanted);
      if (at === 0) prefix.push(option);
      else if (at > 0) inner.push(option);
      else if (hint.indexOf(literal) === 0) byHint.push(option);
    });

    return prefix.concat(inner, byHint);
  }

  /* Returns `[start, end]` of the first case-insensitive literal occurrence of
     `query.trim()` inside `value`, or null when there is none (for instance
     when the match only exists after normalizing). */
  function matchRange(value, query) {
    var needle = String(query == null ? "" : query).trim().toLowerCase();
    if (!needle) return null;
    var at = String(value).toLowerCase().indexOf(needle);
    if (at < 0) return null;
    return [at, at + needle.length];
  }

  /* Turns `input` into a single-value combobox. `config.options()` is called
     on every open/filter and returns `[{ value, hint }]`; `config.emptyText`
     is shown (and announced) when nothing matches. Returns
     `{ refresh, close, destroy }`. */
  function attach(input, config) {
    var doc = input.ownerDocument;
    var win = doc.defaultView;
    var cfg = config || {};
    var id = "typeahead-list-" + ++counter;
    var listbox = doc.createElement("div");
    var status = doc.createElement("span");
    var current = [];
    var active = -1;
    var open = false;
    var choosing = false;
    var savedAttrs = {};

    ["role", "aria-autocomplete", "aria-expanded", "aria-controls", "autocomplete", "list"]
      .forEach(function (name) {
        savedAttrs[name] = input.hasAttribute(name) ? input.getAttribute(name) : null;
      });

    input.setAttribute("role", "combobox");
    input.setAttribute("aria-autocomplete", "list");
    input.setAttribute("aria-expanded", "false");
    input.setAttribute("aria-controls", id);
    input.setAttribute("autocomplete", "off");
    input.removeAttribute("list");

    listbox.className = "typeahead__listbox";
    listbox.setAttribute("role", "listbox");
    listbox.id = id;
    listbox.hidden = true;
    status.className = "visually-hidden";
    status.setAttribute("role", "status");
    doc.body.appendChild(listbox);
    doc.body.appendChild(status);

    function catalogue() {
      var value = typeof cfg.options === "function" ? cfg.options() : [];
      return Array.isArray(value) ? value : [];
    }

    function setActive(index) {
      var nodes = listbox.querySelectorAll(".typeahead__option");
      active = index;
      for (var i = 0; i < nodes.length; i++) {
        var on = i === index;
        nodes[i].classList.toggle("typeahead__option--active", on);
        nodes[i].setAttribute("aria-selected", on ? "true" : "false");
      }
      if (index >= 0 && nodes[index]) {
        input.setAttribute("aria-activedescendant", nodes[index].id);
        if (nodes[index].scrollIntoView) nodes[index].scrollIntoView({ block: "nearest" });
      } else {
        input.removeAttribute("aria-activedescendant");
      }
    }

    /* Placement rule: open below when the list fits below the input; else
       above when it fits above; else on the side with more room, capping the
       listbox's inline max-height to that side's space so it never leaves the
       window. */
    function position() {
      var rect = input.getBoundingClientRect();
      var viewport = win.innerHeight || doc.documentElement.clientHeight;
      var height = listbox.offsetHeight;
      var below = viewport - rect.bottom - GAP;
      var above = rect.top - GAP;
      var up;
      var width = Math.max(rect.width, MIN_WIDTH);
      var viewportWidth = win.innerWidth || doc.documentElement.clientWidth;
      listbox.style.left = Math.max(8, Math.min(rect.left, viewportWidth - width - 8)) + "px";
      listbox.style.width = width + "px";
      listbox.style.maxHeight = "";
      height = listbox.offsetHeight;
      if (height <= below) up = false;
      else if (height <= above) up = true;
      else {
        up = above > below;
        listbox.style.maxHeight = Math.max(up ? above : below, 0) + "px";
      }
      if (up) {
        listbox.style.top = "auto";
        listbox.style.bottom = viewport - rect.top + GAP + "px";
      } else {
        listbox.style.bottom = "auto";
        listbox.style.top = rect.bottom + GAP + "px";
      }
    }

    function buildOption(option, index) {
      var node = doc.createElement("div");
      var label = doc.createElement("span");
      var value = String(option.value);
      var range = matchRange(value, input.value);
      node.className = "typeahead__option";
      node.setAttribute("role", "option");
      node.id = id + "-opt-" + index;
      node.setAttribute("aria-selected", "false");
      label.className = "typeahead__value";
      if (range) {
        var bold = doc.createElement("b");
        bold.textContent = value.slice(range[0], range[1]);
        label.appendChild(doc.createTextNode(value.slice(0, range[0])));
        label.appendChild(bold);
        label.appendChild(doc.createTextNode(value.slice(range[1])));
      } else {
        label.textContent = value;
      }
      node.appendChild(label);
      if (option.hint) {
        var hint = doc.createElement("span");
        hint.className = "typeahead__hint";
        hint.textContent = option.hint;
        node.appendChild(hint);
      }
      node.addEventListener("mousedown", function (event) {
        event.preventDefault();
        choose(index);
      });
      node.addEventListener("mousemove", function () {
        if (active !== index) setActive(index);
      });
      return node;
    }

    function render() {
      var all = catalogue();
      if (!all.length && !input.value.trim()) {
        close();
        return;
      }
      current = filterOptions(all, input.value);
      while (listbox.firstChild) listbox.removeChild(listbox.firstChild);
      if (current.length) {
        current.forEach(function (option, index) {
          listbox.appendChild(buildOption(option, index));
        });
        status.textContent = "";
      } else {
        var empty = doc.createElement("div");
        empty.className = "typeahead__empty";
        empty.setAttribute("role", "presentation");
        empty.textContent = cfg.emptyText || "";
        listbox.appendChild(empty);
        status.textContent = cfg.emptyText || "";
      }
      listbox.hidden = false;
      open = true;
      input.setAttribute("aria-expanded", "true");
      setActive(-1);
      position();
    }

    function close() {
      if (!open && listbox.hidden) return;
      open = false;
      listbox.hidden = true;
      active = -1;
      input.setAttribute("aria-expanded", "false");
      input.removeAttribute("aria-activedescendant");
    }

    function choose(index) {
      var option = current[index];
      if (!option) return;
      input.value = option.value;
      close();
      choosing = true;
      try {
        input.dispatchEvent(new win.Event("input", { bubbles: true }));
        input.dispatchEvent(new win.Event("change", { bubbles: true }));
      } finally {
        choosing = false;
      }
    }

    function onInput() {
      if (choosing) return;
      render();
    }

    function onListboxMousedown(event) {
      event.preventDefault();
    }

    function move(step) {
      if (!open) {
        render();
        if (!open) return;
      }
      if (!current.length) return;
      var next = active < 0 ? (step > 0 ? 0 : current.length - 1) : active + step;
      if (next >= current.length) next = 0;
      if (next < 0) next = current.length - 1;
      setActive(next);
    }

    function onKeydown(event) {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        move(1);
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        move(-1);
      } else if (event.key === "Enter") {
        if (open && active >= 0) {
          event.preventDefault();
          choose(active);
        }
      } else if (event.key === "Escape") {
        if (open) {
          event.stopPropagation();
          close();
        }
      } else if (event.key === "Tab") {
        close();
      }
    }

    function onScroll(event) {
      if (!open) return;
      if (event.target === input) return;
      if (event.target && listbox.contains(event.target)) return;
      close();
    }

    function onResize() {
      close();
    }

    /* A click on an already focused input reopens a closed list (the focus
       event does not fire again). Option mousedown is default-prevented, so
       choosing with the mouse never produces a click on the input. */
    function onClick() {
      if (!open) render();
    }

    input.addEventListener("focus", render);
    input.addEventListener("click", onClick);
    input.addEventListener("input", onInput);
    listbox.addEventListener("mousedown", onListboxMousedown);
    input.addEventListener("keydown", onKeydown);
    input.addEventListener("blur", close);
    win.addEventListener("scroll", onScroll, true);
    win.addEventListener("resize", onResize);

    /* Re-reads the catalogue and re-filters while the list is open. */
    function refresh() {
      if (open) render();
    }

    /* Hides the list without touching the input's value. */
    function closePublic() {
      close();
    }

    /* Removes listeners, ARIA attributes added by `attach`, and the listbox
       and status nodes. */
    function destroy() {
      input.removeEventListener("focus", render);
      input.removeEventListener("click", onClick);
      input.removeEventListener("input", onInput);
      listbox.removeEventListener("mousedown", onListboxMousedown);
      input.removeEventListener("keydown", onKeydown);
      input.removeEventListener("blur", close);
      win.removeEventListener("scroll", onScroll, true);
      win.removeEventListener("resize", onResize);
      input.removeAttribute("aria-activedescendant");
      Object.keys(savedAttrs).forEach(function (name) {
        if (savedAttrs[name] === null) input.removeAttribute(name);
        else input.setAttribute(name, savedAttrs[name]);
      });
      if (listbox.parentNode) listbox.parentNode.removeChild(listbox);
      if (status.parentNode) status.parentNode.removeChild(status);
    }

    return { refresh: refresh, close: closePublic, destroy: destroy };
  }

  return {
    normalize: normalize,
    filterOptions: filterOptions,
    matchRange: matchRange,
    attach: attach,
  };
});
