/*
 * Type-rail tooltip and reveal-on-arrival (#238, generalized for #603).
 *
 * Progressive enhancement only over a fully server-rendered rail. The
 * "Recently used" group itself — including its clicks' write-back to
 * `rails.<page>` — is no longer this script's job (#754/#755): the server
 * renders the group from `rails.<page>.recent` (`partials/rail_recent.html`),
 * and on the pages where a rail click is intercepted in-page,
 * `saved-queries.js` repaints the group locally and records the click, the
 * same way it already owns the rest of that in-page navigation. This script
 * keeps only the two behaviors that apply to every rail item regardless of
 * where it came from — the server-rendered list or the server-rendered
 * "Recently used" group — since both render the identical
 * `a.filter-rail__item` shape and both are covered by the delegated
 * selectors below:
 *   - an accessible tooltip for a label the rail's fixed width clips;
 *   - scrolling the list so the selected item is visible on arrival.
 */

/* A native `title` exposes truncated names to a pointer, but not visibly to a
   keyboard user. Replace that fallback with one body-level tooltip whenever
   a rail or resource-grid label is genuinely clipped (#634, #793). Keeping
   the tooltip outside scrolling and grid containers avoids overflow clipping,
   and delegation means dynamically added items follow the same path. */
(function () {
  "use strict";

  var ITEM_SELECTOR = [
    "a.filter-rail__item[data-full-name]",
    "label.typegrid__item[data-full-name]",
  ].join(", ");
  var tooltip = document.createElement("div");
  var activeItem = null;
  var activeTrigger = null;
  var hoveredItem = null;
  var focusedItem = null;
  var focusedTrigger = null;
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
    if (activeTrigger) activeTrigger.removeAttribute("aria-describedby");
    activeItem = null;
    activeTrigger = null;
    tooltip.hidden = true;
  }

  function isClipped(label) {
    if (label.scrollWidth > label.clientWidth) return true;

    /* scrollWidth/clientWidth are integer-rounded in Chromium, while the
       ellipsis decision is made from fractional layout geometry. Measure the
       text run itself so sub-pixel clipping still receives a tooltip (#793). */
    var range = document.createRange();
    range.selectNodeContents(label);
    return range.getBoundingClientRect().width > label.getBoundingClientRect().width + 0.01;
  }

  function show(item) {
    var label = item && item.querySelector(".filter-rail__label, .typegrid__label");
    var fullName = item && item.getAttribute("data-full-name");
    var trigger = focusedItem === item && focusedTrigger
      ? focusedTrigger
      : item && item.matches("a.filter-rail__item")
        ? item
        : item && item.querySelector('input[type="checkbox"]');
    /* HTMX can replace Search Parameters rail items after the initial sweep;
       remove their fallback lazily as well. */
    if (item) item.removeAttribute("title");
    if (!label || !fullName || !trigger || !isClipped(label)) {
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

    if (activeTrigger && activeTrigger !== trigger) {
      activeTrigger.removeAttribute("aria-describedby");
    }
    activeItem = item;
    activeTrigger = trigger;
    activeTrigger.setAttribute("aria-describedby", tooltip.id);
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
      && (
        !focusedItem.isConnected
        || !focusedTrigger
        || !focusedTrigger.isConnected
        || document.activeElement !== focusedTrigger
      )
    ) {
      focusedItem = null;
      focusedTrigger = null;
    }
    if (
      hoveredItem
      && (!hoveredItem.isConnected || !hoveredItem.matches(":hover"))
    ) {
      hoveredItem = null;
    }
    /* Pointer intent wins while it is over an item; when it leaves, the
       still-focused control resumes its keyboard tooltip. */
    var item = hoveredItem || focusedItem;
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
    focusedTrigger = event.target instanceof Element ? event.target : null;
    refresh();
  });

  document.addEventListener("focusout", function (event) {
    var item = closestItem(event.target);
    if (!item) return;
    if (focusedItem === item) {
      focusedItem = null;
      focusedTrigger = null;
    }
    refresh();
  });

  document.addEventListener("change", refresh);
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
