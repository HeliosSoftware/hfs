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
