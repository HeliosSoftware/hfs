/* Dashboard chart tooltip (#555): a hover readout over the server-rendered
   SVG. The inert #chart-data carrier holds the bucket labels and every plotted
   series' values with their SVG coordinates — the server did the chart math,
   this script only maps the pointer to the nearest bucket and shows what is
   there. Without JavaScript the chart, its legend, and the tabular alternative
   are complete; this only adds the readout. init() is re-run whenever the
   view-all toggle (#599) swaps in a fresh chart card, and whenever htmx
   settles the pending dashboard's #dash-live region (#956) — either swap
   brings new #chart-wrap/#chart-tip/#chart-data nodes, so the listeners bound
   below need rebinding to them. A flag on the wrapper makes re-running init()
   over an already-bound chart a no-op, so the two triggers cannot stack a
   second guide element and a second set of listeners on one node. */
(function () {
  "use strict";

  function init() {
    var wrap = document.getElementById("chart-wrap");
    var tip = document.getElementById("chart-tip");
    var carrier = document.getElementById("chart-data");
    if (!wrap || !tip || !carrier) return;
    if (wrap.dataset.tipBound) return;
    wrap.dataset.tipBound = "1";

    var data;
    try {
      data = JSON.parse(carrier.textContent);
    } catch (invalid) {
      return;
    }
    if (!data.xs || !data.xs.length || !data.series || !data.series.length) return;

    var svg = wrap.querySelector("svg.chart");
    if (!svg) return;

    var guide = document.createElement("div");
    guide.className = "chart-guide";
    guide.hidden = true;
    wrap.appendChild(guide);

    wrap.addEventListener("mousemove", function (event) {
      var rect = svg.getBoundingClientRect();
      var box = svg.viewBox.baseVal;
      if (!rect.width || !box.width) return;
      var vx = ((event.clientX - rect.left) * box.width) / rect.width;

      var nearest = 0;
      var distance = Infinity;
      for (var i = 0; i < data.xs.length; i++) {
        var d = Math.abs(data.xs[i] - vx);
        if (d < distance) {
          distance = d;
          nearest = i;
        }
      }

      tip.textContent = "";
      var date = document.createElement("div");
      date.className = "chart-tip__date";
      date.textContent = data.labels[nearest];
      tip.appendChild(date);
      data.series.forEach(function (s) {
        var row = document.createElement("div");
        row.className = "chart-tip__row";
        var name = document.createElement("span");
        name.className = "chart-tip__name";
        var dot = document.createElement("span");
        dot.className = "chart-legend__dot chart-legend__dot--" + s.color;
        dot.setAttribute("aria-hidden", "true");
        name.appendChild(dot);
        name.appendChild(document.createTextNode(s.type));
        var value = document.createElement("span");
        value.className = "chart-tip__value";
        value.textContent = Number(s.values[nearest]).toLocaleString();
        row.appendChild(name);
        row.appendChild(value);
        tip.appendChild(row);
      });

      var wrapRect = wrap.getBoundingClientRect();
      var px = rect.left - wrapRect.left + (data.xs[nearest] * rect.width) / box.width;
      guide.style.left = px + "px";
      guide.style.top = rect.top - wrapRect.top + "px";
      guide.style.height = rect.height + "px";
      guide.hidden = false;

      tip.hidden = false;
      var tipX = px + 12;
      if (tipX + tip.offsetWidth > wrap.clientWidth) tipX = px - tip.offsetWidth - 12;
      tip.style.left = Math.max(0, tipX) + "px";
      tip.style.top = "18px";
    });

    wrap.addEventListener("mouseleave", function () {
      tip.hidden = true;
      guide.hidden = true;
    });
  }

  init();
  document.addEventListener("hfs:chart-swapped", init);
  document.addEventListener("htmx:afterSettle", init);
})();

/* Type-pick options (#599, extended): every option row in the picker — the
   individual type toggles and the "view all resources" row alike — is a
   plain link that flips the charted set (or the all-types flag) via a
   query-string change. Without JavaScript any of them navigates and
   everything still works; the <details> picker just collapses because the
   whole page re-renders, which is awkward when the user's very next action
   is usually another pick from the same menu. With JavaScript, swap in just
   the chart card instead of the full page: fetch the same href, parse the
   response, and replace section.card.chart-card with the response's copy.
   That section carries the picker, window selector, chart, table and
   legend — every option's href lives inside it — so one swap keeps them all
   coherent (including the all-types flag surviving window/type changes)
   without the full-page flicker. Reopen the picker menu on the new node
   afterward, since the swap replaces the one the user had open, and carry
   over anything typed into the picker filter — re-dispatching an "input"
   event lets the filter IIFE below do the actual hiding, rather than
   duplicating its logic here. Dispatch "hfs:chart-swapped" afterward so the
   tooltip IIFE above can rebind to the fresh
   #chart-wrap/#chart-tip/#chart-data it just brought in. The swap also
   replaces the node this handler would otherwise be bound to, so the
   listener is delegated on document rather than the picker itself. Falls
   back to a plain navigation on any fetch or parse failure. */
(function () {
  "use strict";
  document.addEventListener("click", function (event) {
    var toggle = event.target.closest ? event.target.closest("a.chart-pick__option") : null;
    if (!toggle) return;
    event.preventDefault();

    var href = toggle.href;
    var filterBefore = document.querySelector("[data-pick-filter]");
    var filterValue = filterBefore ? filterBefore.value : "";

    // Marks the swap in flight so the #dash-live refresh (below) does not
    // re-render the region under it (#1078).
    document.documentElement.setAttribute("data-dash-picking", "");
    fetch(href)
      .then(function (response) {
        if (!response.ok) throw new Error("unexpected response");
        return response.text();
      })
      .then(function (html) {
        var doc = new DOMParser().parseFromString(html, "text/html");
        var next = doc.querySelector("section.card.chart-card");
        var current = document.querySelector("section.card.chart-card");
        if (!next || !current) throw new Error("chart card missing from response");
        current.replaceWith(next);
        var pick = next.querySelector("details.chart-pick");
        if (pick) pick.open = true;
        if (filterValue) {
          var filterAfter = next.querySelector("[data-pick-filter]");
          if (filterAfter) {
            filterAfter.value = filterValue;
            filterAfter.dispatchEvent(new Event("input", { bubbles: true }));
            filterAfter.focus();
          }
        }
        history.pushState(null, "", href);
        document.documentElement.removeAttribute("data-dash-picking");
        document.dispatchEvent(new CustomEvent("hfs:chart-swapped"));
      })
      .catch(function () {
        document.documentElement.removeAttribute("data-dash-picking");
        window.location = href;
      });
  });
})();

/* Live refresh (#1078): while the dashboard's figures are still moving —
   approximate, or with an import running — the server renders #dash-live
   with `hx-trigger="every Ns [hfsDashCanRefresh()]"` and data-dash-refresh,
   and stops rendering them once the figures settle. The point is that the
   figures keep climbing on their own, so the refresh must not stall while the
   user is merely looking at or using the dashboard. Instead of skipping ticks,
   each swap carries the user's state across:

   - The open type picker is kept as the very same node (hx-preserve is added
     to the response's #chart-pick just before the swap), so it stays open with
     its filter text, list scroll and focus; its counts update once it closes.
   - An open data table (#chart-table) is re-opened in the response.
   - A tooltip showing when the refresh lands is re-shown for the pointer's
     position over the new chart.
   - Returning to the tab refreshes at once instead of waiting for a tick.

   hfsDashCanRefresh() only skips a tick while the tab is hidden, a picker swap
   is in flight, or keyboard focus sits in the region outside the picker — an
   outerHTML swap would drop a keyboard user's place, while a mouse click's
   leftover focus is harmless to lose.

   The type picker swaps only the chart card and pushState()s its URL, so the
   poll's hx-get is left pointing at the old selection: each refresh request is
   re-aimed at the current location, and a response is dropped if the location
   changed while it was in flight. The request also names the notice kinds on
   screen (?notices=), so the server renders those lines aria-live="off": an
   unchanged "approximate" line is not re-announced every tick. */
(function () {
  "use strict";

  var pointer = null;
  document.addEventListener("mousemove", function (event) {
    pointer = { x: event.clientX, y: event.clientY };
  });

  function isRefresh(elt) {
    return !!elt && elt.id === "dash-live" && elt.hasAttribute("data-dash-refresh");
  }

  function here() {
    return window.location.pathname + window.location.search;
  }

  window.hfsDashCanRefresh = function () {
    if (document.hidden) return false;
    if (document.documentElement.hasAttribute("data-dash-picking")) return false;
    var live = document.getElementById("dash-live");
    if (!live) return false;
    var active = document.activeElement;
    if (
      active &&
      active !== document.body &&
      live.contains(active) &&
      !active.closest("#chart-pick") &&
      active.matches(":focus-visible")
    ) {
      return false;
    }
    return true;
  };

  document.addEventListener("htmx:configRequest", function (event) {
    var elt = event.detail.elt;
    if (!isRefresh(elt)) return;
    var path = here();
    event.detail.path = path;
    var seen = [];
    elt.querySelectorAll("[data-dash-notice]").forEach(function (line) {
      seen.push(line.getAttribute("data-dash-notice"));
    });
    if (seen.length) event.detail.parameters.notices = seen.join(",");
    elt.setAttribute("data-dash-requested", path);
  });

  document.addEventListener("htmx:beforeSwap", function (event) {
    var elt = event.detail.elt;
    if (!isRefresh(elt)) return;
    if (elt.getAttribute("data-dash-requested") !== here()) {
      event.detail.shouldSwap = false;
      return;
    }
    var html = event.detail.serverResponse;
    if (typeof html !== "string") return;
    var pick = document.getElementById("chart-pick");
    var state = { tip: false, focus: null, selection: null, scrolls: [] };
    if (pick && pick.open) {
      html = html.replace('id="chart-pick"', 'id="chart-pick" hx-preserve');
      // htmx keeps the node, but moving it can drop focus, caret and list
      // scroll, so they are put back after the swap.
      var active = document.activeElement;
      if (active && pick.contains(active)) {
        state.focus = active;
        if (typeof active.selectionStart === "number") {
          state.selection = [active.selectionStart, active.selectionEnd];
        }
      }
      pick.querySelectorAll("*").forEach(function (node) {
        if (node.scrollTop > 0) state.scrolls.push([node, node.scrollTop]);
      });
    }
    var table = document.getElementById("chart-table");
    if (table && table.open) {
      html = html.replace('id="chart-table"', 'id="chart-table" open');
    }
    var tip = document.getElementById("chart-tip");
    state.tip = !!(tip && !tip.hidden);
    // Kept here, not on #dash-live: the swap throws that node away.
    carried = state;
    event.detail.serverResponse = html;
  });

  // State carried across the refresh swap in flight, set just before it.
  var carried = null;

  // Runs after the tooltip IIFE's own afterSettle listener has rebound the new
  // chart, so the synthetic move lands on a live handler.
  document.addEventListener("htmx:afterSettle", function () {
    if (!carried) return;
    var state = carried;
    carried = null;
    state.scrolls.forEach(function (entry) {
      if (entry[0].isConnected) entry[0].scrollTop = entry[1];
    });
    var focus = state.focus;
    if (focus && focus.isConnected && document.activeElement !== focus) {
      focus.focus({ preventScroll: true });
      if (state.selection && typeof focus.setSelectionRange === "function") {
        try {
          focus.setSelectionRange(state.selection[0], state.selection[1]);
        } catch (unsupported) {
          /* not a text field */
        }
      }
    }
    if (!state.tip || !pointer) return;
    var wrap = document.getElementById("chart-wrap");
    if (!wrap || !wrap.matches(":hover")) return;
    wrap.dispatchEvent(
      new MouseEvent("mousemove", { clientX: pointer.x, clientY: pointer.y, bubbles: true })
    );
  });

  document.addEventListener("visibilitychange", function () {
    if (document.hidden || !window.htmx) return;
    var live = document.getElementById("dash-live");
    if (!isRefresh(live) || !window.hfsDashCanRefresh()) return;
    window.htmx.ajax("GET", here(), {
      source: live,
      target: live,
      select: "#dash-live",
      swap: "outerHTML",
    });
  });
})();

/* Type-picker filter: typeahead over the option rows, same pattern as the
   resource rail's filter. Without JavaScript the list simply scrolls. */
(function () {
  "use strict";
  document.addEventListener("input", function (event) {
    var filter = event.target.closest ? event.target.closest("[data-pick-filter]") : null;
    if (!filter) return;
    var panel = filter.closest(".menu__panel");
    if (!panel) return;
    var needle = filter.value.trim().toLowerCase();
    panel.querySelectorAll("[data-pick-name]").forEach(function (option) {
      option.hidden = !!needle && option.dataset.pickName.toLowerCase().indexOf(needle) === -1;
    });
  });
})();
