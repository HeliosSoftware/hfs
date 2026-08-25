/*
 * Type-rail "Recently used" group (#238, generalized for #603).
 *
 * Progressive enhancement only — the rail is fully server-rendered and works
 * without this script; recents are a per-browser convenience kept in
 * localStorage (`data-recent-key` on the group container). The group stays
 * hidden until it has at least one entry, and entries are clones of the
 * server-rendered rail items so hrefs, counts, and i18n come from the page,
 * never from here.
 *
 * The group container's `data-*` attributes configure where clones come
 * from, so the same script drives both Search Parameters and Resources
 * without hardcoding either page's ids:
 *   - data-recent-key: localStorage key. Shared (`hfs-recent-types`) by
 *     default across pages, so recents travel between Resources and Search
 *     Parameters.
 *   - data-rail-list: id of the source list to clone items from. Defaults to
 *     Search Parameters' `sp-rail-list` so that page needs no markup change.
 *   - data-rail-item: CSS selector for one rail item. Defaults to
 *     `a.filter-rail__item` (#541's shared rail markup).
 *   - data-rail-attr: attribute holding the type name. Defaults to
 *     `data-type`.
 *
 * Click registration is scoped to items inside the source list or the group
 * itself (never a bare `[data-type]` at document level) so unrelated
 * controls that happen to carry `data-type` — "Create new", the delete
 * button in the Search Parameters detail pane — never get recorded as
 * recents.
 */
(function () {
  "use strict";

  var MAX_RECENT = 5;
  var DEFAULT_LIST_ID = "sp-rail-list";
  var DEFAULT_ITEM_SELECTOR = "a.filter-rail__item";
  var DEFAULT_ATTR = "data-type";

  var group = document.querySelector("[data-recent-key]");
  if (!group) return;
  var key = group.getAttribute("data-recent-key");
  var itemSelector = group.getAttribute("data-rail-item") || DEFAULT_ITEM_SELECTOR;
  var attrName = group.getAttribute("data-rail-attr") || DEFAULT_ATTR;
  var list = document.getElementById(group.getAttribute("data-rail-list") || DEFAULT_LIST_ID);
  if (!list) return;

  function readRecents() {
    try {
      var parsed = JSON.parse(localStorage.getItem(key) || "[]");
      return Array.isArray(parsed) ? parsed.filter(function (n) { return typeof n === "string"; }) : [];
    } catch (e) {
      return [];
    }
  }

  function writeRecents(names) {
    try {
      localStorage.setItem(key, JSON.stringify(names.slice(0, MAX_RECENT)));
    } catch (e) {
      /* Private mode / quota: recents just don't persist. */
    }
  }

  function renderRecents() {
    group.querySelectorAll(itemSelector).forEach(function (n) { n.remove(); });
    var shown = 0;
    readRecents().forEach(function (name) {
      var item = list.querySelector("[" + attrName + '="' + CSS.escape(name) + '"]');
      if (!item) return;
      group.appendChild(item.cloneNode(true));
      shown++;
    });
    group.hidden = shown === 0;
  }

  document.addEventListener("click", function (event) {
    var item = event.target.closest(itemSelector);
    if (!item || (!list.contains(item) && !group.contains(item))) return;
    var name = item.getAttribute(attrName);
    if (!name) return;
    var recents = readRecents().filter(function (n) { return n !== name; });
    recents.unshift(name);
    writeRecents(recents);
  });

  renderRecents();
})();

/* A native `title` exposes truncated names to a pointer, but not visibly to a
   keyboard user. Replace that fallback with one body-level tooltip whenever
   a rail label is genuinely clipped (#634). Keeping the tooltip outside the
   scrolling rail avoids overflow clipping, and delegation means the cloned
   Recently used items follow the same path without duplicate ids. */
(function () {
  "use strict";

  var ITEM_SELECTOR = "a.filter-rail__item[data-full-name]";
  var tooltip = document.createElement("div");
  var activeItem = null;
  var hoveredItem = null;
  var focusedItem = null;
  tooltip.className = "filter-rail__tooltip";
  tooltip.id = "filter-rail-tooltip";
  tooltip.setAttribute("role", "tooltip");
  tooltip.hidden = true;
  document.body.appendChild(tooltip);

  /* `title` remains useful when JavaScript is unavailable. Once this richer
     tooltip is active, remove it to avoid two competing hover bubbles. */
  document.querySelectorAll(ITEM_SELECTOR).forEach(function (item) {
    item.removeAttribute("title");
  });

  function hide(item) {
    if (item && item !== activeItem) return;
    if (activeItem) activeItem.removeAttribute("aria-describedby");
    activeItem = null;
    tooltip.hidden = true;
  }

  function show(item) {
    var label = item && item.querySelector(".filter-rail__label");
    var fullName = item && item.getAttribute("data-full-name");
    /* HTMX can replace Search Parameters rail items after the initial sweep;
       remove their fallback lazily as well. */
    if (item) item.removeAttribute("title");
    if (!label || !fullName || label.scrollWidth <= label.clientWidth + 1) {
      hide();
      return;
    }

    var itemRect = item.getBoundingClientRect();
    if (
      itemRect.bottom <= 0
      || itemRect.top >= window.innerHeight
      || itemRect.right <= 0
      || itemRect.left >= window.innerWidth
    ) {
      hide();
      return;
    }

    if (activeItem && activeItem !== item) {
      activeItem.removeAttribute("aria-describedby");
    }
    activeItem = item;
    activeItem.setAttribute("aria-describedby", tooltip.id);
    tooltip.textContent = fullName;
    tooltip.hidden = false;

    var tooltipRect = tooltip.getBoundingClientRect();
    var gap = 8;
    var rightFits = itemRect.right + gap + tooltipRect.width <= window.innerWidth - gap;
    var leftFits = itemRect.left - gap - tooltipRect.width >= gap;
    var left;
    var top;

    if (rightFits || leftFits) {
      left = rightFits
        ? itemRect.right + gap
        : itemRect.left - tooltipRect.width - gap;
      top = Math.min(
        Math.max(gap, itemRect.top + (itemRect.height - tooltipRect.height) / 2),
        window.innerHeight - tooltipRect.height - gap
      );
    } else {
      /* In the stacked <=1100px layout there may be no room beside the rail.
         Place the tooltip below (or above) so it never covers the trigger or
         its right-aligned count. */
      left = Math.min(
        Math.max(gap, itemRect.left),
        window.innerWidth - tooltipRect.width - gap
      );
      top = itemRect.bottom + gap;
      if (top + tooltipRect.height > window.innerHeight - gap) {
        top = Math.max(gap, itemRect.top - tooltipRect.height - gap);
      }
    }

    tooltip.style.left = left + "px";
    tooltip.style.top = top + "px";
  }

  function closestItem(target) {
    return target instanceof Element ? target.closest(ITEM_SELECTOR) : null;
  }

  function refresh() {
    if (
      focusedItem
      && (!focusedItem.isConnected || document.activeElement !== focusedItem)
    ) {
      focusedItem = null;
    }
    if (
      hoveredItem
      && (!hoveredItem.isConnected || !hoveredItem.matches(":hover"))
    ) {
      hoveredItem = null;
    }
    var item = focusedItem || hoveredItem;
    if (item) show(item);
    else hide();
  }

  document.addEventListener("mouseover", function (event) {
    var item = closestItem(event.target);
    if (!item) return;
    hoveredItem = item;
    refresh();
  });

  document.addEventListener("mouseout", function (event) {
    var item = closestItem(event.target);
    var related = event.relatedTarget;
    if (!item || (related instanceof Node && item.contains(related))) return;
    if (hoveredItem === item) hoveredItem = null;
    refresh();
  });

  document.addEventListener("focusin", function (event) {
    var item = closestItem(event.target);
    if (!item) return;
    focusedItem = item;
    refresh();
  });

  document.addEventListener("focusout", function (event) {
    var item = closestItem(event.target);
    if (!item) return;
    if (focusedItem === item) focusedItem = null;
    refresh();
  });

  window.addEventListener("resize", refresh);
  var scrollFrame = null;
  document.addEventListener("scroll", function () {
    if (scrollFrame !== null) return;
    scrollFrame = window.requestAnimationFrame(function () {
      scrollFrame = null;
      refresh();
    });
  }, true);
})();

/* Reveal the selected type on arrival: the rail list scrolls itself so the
   `aria-current` item (the deep-linked type, or the default Patient the page
   scripts mark on load) sits near the middle of the list instead of below
   the fold. Runs on window load, after the deferred page scripts have marked
   the selection. Recent clones live outside the scrolling list. */
(function () {
  "use strict";
  window.addEventListener("load", function () {
    document.querySelectorAll(".filter-rail__list").forEach(function (list) {
      var current = list.querySelector('[aria-current="true"]');
      if (!current) return;
      var offset =
        current.getBoundingClientRect().top - list.getBoundingClientRect().top;
      var target = list.scrollTop + offset - (list.clientHeight - current.offsetHeight) / 2;
      if (target > 0) list.scrollTop = target;
    });
  });
})();
