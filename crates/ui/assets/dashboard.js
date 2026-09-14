/* Dashboard chart tooltip (#555): a hover readout over the server-rendered
   SVG. The inert #chart-data carrier holds the bucket labels and every plotted
   series' values with their SVG coordinates — the server did the chart math,
   this script only maps the pointer to the nearest bucket and shows what is
   there. Without JavaScript the chart, its legend, and the tabular alternative
   are complete; this only adds the readout. init() is re-run whenever htmx
   settles a swap — a type-picker option (#599) swapping in a fresh #dash-chart
   card, or the dashboard's #dash-live region refreshing (#956, #1078) — and
   whenever htmx restores a page from its history cache: each brings new
   #chart-wrap/#chart-tip/#chart-data nodes, so the listeners bound below need
   rebinding to them. A property on the wrapper (not an attribute, which htmx's
   history snapshot would keep while the listeners are lost) makes re-running
   init() over an already-bound chart a no-op, so the triggers cannot stack a
   second set of listeners on one node; a guide element a snapshot brought
   back is reused rather than doubled. */
(function () {
  "use strict";

  function init() {
    var wrap = document.getElementById("chart-wrap");
    var tip = document.getElementById("chart-tip");
    var carrier = document.getElementById("chart-data");
    if (!wrap || !tip || !carrier) return;
    if (wrap.hfsTipBound) return;
    wrap.hfsTipBound = true;

    var data;
    try {
      data = JSON.parse(carrier.textContent);
    } catch (invalid) {
      return;
    }
    if (!data.xs || !data.xs.length || !data.series || !data.series.length) return;

    var svg = wrap.querySelector("svg.chart");
    if (!svg) return;

    var guide = wrap.querySelector(".chart-guide");
    if (!guide) {
      guide = document.createElement("div");
      guide.className = "chart-guide";
      wrap.appendChild(guide);
    }
    guide.hidden = true;
    tip.hidden = true;

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
  document.addEventListener("htmx:afterSettle", init);
  document.addEventListener("htmx:historyRestore", init);
})();

/* Live refresh and the type picker (#1078, #599, #956): both are plain htmx;
   this script only carries the user's state across their swaps. The server
   answers each request by its HX-Target.

   The refresh. Every ready dashboard renders #dash-live with
   `hx-trigger="every Ns [hfsDashCanRefresh()]"`, `hx-sync="this:drop"`,
   data-dash-refresh and a data-dash-state digest of its figures — every few
   seconds while they are still moving (approximate, or an import running:
   data-dash-moving), slower once they settle, so a tab opened before an
   import starts still notices it. The request (HX-Target: dash-live) gets
   only the #dash-live fragment back. The figures must keep climbing on their
   own while the user is looking at or using the dashboard, so ticks are not
   skipped; instead each request tells the server what is on screen, and the
   server renders it back that way:

   - ?state= is the digest on screen, sent only while the region is settled
     (no data-dash-moving, no data-dash-waiting). When the figures are still
     the same and still settled the server answers 204 and htmx swaps
     nothing, so a quiet page is left untouched.
   - ?open=pick,table names the open type picker and data table. The server
     renders the picker `open hx-preserve` — htmx keeps the very node, with
     its filter text and option states — and the table `open`. Moving the
     kept node can still drop focus, the filter's caret and the list's
     scroll, so they are saved before the swap and put back after it.
   - ?notices= names the notice kinds on screen, so the server renders those
     lines aria-live="off": an unchanged "approximate" line is not
     re-announced every tick.
   - A tooltip showing when the refresh lands is re-shown for the pointer's
     position over the new chart. Returning to the tab refreshes at once
     instead of waiting for a tick.

   hfsDashCanRefresh() only skips a tick while the tab is hidden or keyboard
   focus sits in the region outside the picker — an outerHTML swap would drop
   a keyboard user's place, while a mouse click's leftover focus is harmless
   to lose. A picker request in flight needs no flag: hx-sync lets it abort
   the tick, and drops a tick that fires during it.

   The type picker. Each option (the "view all resources" row included) is a
   link that still navigates without JavaScript; htmx GETs the same href into
   #dash-chart (HX-Target: dash-chart, `hx-sync="#dash-live:replace"`). The
   server sends the chart card alone with the picker open and HX-Push-Url set
   to the selection's own link, which htmx pushes — so Back restores the
   previous selection from htmx's history. The filter input is hx-preserve in
   every render except inside a preserved picker (which carries it along;
   nesting would let htmx pull it out of that picker), so its text survives
   either swap; the filter script below
   re-applies it to the new option rows. Focus (on the clicked option, found
   again by its type in the new list) and the filter's caret are carried the
   same way as for the refresh. htmx's history snapshot is the live page, so
   the picker's hx-preserve is dropped from it before it is saved: restored,
   it would keep the current picker in place of the one being gone back to.

   The picker changes the URL without touching #dash-live's hx-get, so each
   refresh request is re-aimed at the current location, and a response is
   dropped if the location changed while it was in flight.

   A waiting page whose fast retries are spent keeps a slow watch instead
   (data-dash-waiting): its re-aimed request carries the spent retry count
   from its own hx-get, so the server answers with the watch again rather than
   restarting the fast retries, until the figures arrive.

   Every request the region makes — refresh, slow watch, bounded retry or
   picker option — also sends #dash-live's data-dash-ctx back as ?ctx=
   (tenant|FHIR version|locale). When another tab switched any of them, the
   server answers HX-Refresh and htmx reloads the whole page, so one context's
   figures never land in another's page. */
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

  // The value of `name` in a relative href's query string, or null.
  function queryParam(href, name) {
    try {
      return new URL(href, window.location.origin).searchParams.get(name);
    } catch (invalid) {
      return null;
    }
  }

  document.addEventListener("htmx:configRequest", function (event) {
    var elt = event.detail.elt;
    if (!elt || !elt.closest) return;
    var live = elt.closest("#dash-live");
    if (!live) return;
    var params = event.detail.parameters;
    var ctx = live.getAttribute("data-dash-ctx");
    if (ctx) params.ctx = ctx;
    // A control inside the region (a picker option) sends nothing else: its
    // href is the selection, and the server pushes the canonical URL.
    if (elt !== live) return;

    var open = [];
    var pick = live.querySelector("#chart-pick");
    if (pick && pick.open) open.push("pick");
    var table = live.querySelector("#chart-table");
    if (table && table.open) open.push("table");
    if (open.length) params.open = open.join(",");

    if (!isRefresh(elt)) return;
    var path = here();
    // The location may still name a retry count (the "Retry now" link sets
    // one); the watch decides its own, below, so it is not sent twice. Only
    // that pair is dropped, the rest kept byte for byte: URLSearchParams would
    // re-encode the query and turn ?types=A,B into A%2CB, which the server
    // reads raw — the refresh would then chart the default types instead.
    var kept = window.location.search
      .replace(/^\?/, "")
      .split("&")
      .filter(function (pair) {
        return pair && pair.split("=")[0] !== "retry";
      });
    event.detail.path = window.location.pathname + (kept.length ? "?" + kept.join("&") : "");
    var waiting = elt.hasAttribute("data-dash-waiting");
    if (waiting) {
      var retry = queryParam(elt.getAttribute("hx-get") || "", "retry");
      if (retry) params.retry = retry;
    }
    var state = elt.getAttribute("data-dash-state");
    if (state && !waiting && !elt.hasAttribute("data-dash-moving")) params.state = state;
    var seen = [];
    elt.querySelectorAll("[data-dash-notice]").forEach(function (line) {
      seen.push(line.getAttribute("data-dash-notice"));
    });
    if (seen.length) params.notices = seen.join(",");
    elt.setAttribute("data-dash-requested", path);
  });

  // State carried across the swap in flight, by swap target ("dash-live" or
  // "dash-chart"). Kept here, not on the target: the swap throws it away.
  var carried = {};

  // Which option an element sits in, as a key that finds its counterpart in
  // a freshly rendered list: "all" for "view all resources", else its type.
  function optionKey(node) {
    var option = node.closest("a.chart-pick__option");
    if (!option) return null;
    if (option.classList.contains("chart-pick__option--all")) return "all";
    return option.getAttribute("data-pick-name");
  }

  function optionByKey(pick, key) {
    if (key === "all") return pick.querySelector("a.chart-pick__option--all");
    var options = pick.querySelectorAll("a.chart-pick__option[data-pick-name]");
    for (var i = 0; i < options.length; i++) {
      if (options[i].getAttribute("data-pick-name") === key) return options[i];
    }
    return null;
  }

  function capture() {
    var state = { tip: false, focus: null, option: null, selection: null, scrolls: [] };
    var pick = document.getElementById("chart-pick");
    if (pick && pick.open) {
      var active = document.activeElement;
      if (active && pick.contains(active)) {
        state.focus = active;
        state.option = optionKey(active);
        if (typeof active.selectionStart === "number") {
          state.selection = [active.selectionStart, active.selectionEnd];
        }
      }
      // By node, and by class for when the swap replaced the node (a picker
      // swap renders a new option list).
      pick.querySelectorAll("*").forEach(function (node) {
        if (node.scrollTop <= 0) return;
        var classes = Array.prototype.slice.call(node.classList);
        state.scrolls.push([node, node.scrollTop, classes.length ? "." + classes.join(".") : null]);
      });
    }
    var tip = document.getElementById("chart-tip");
    state.tip = !!(tip && !tip.hidden);
    return state;
  }

  document.addEventListener("htmx:beforeSwap", function (event) {
    var target = event.detail.target;
    var key = target && target.id;
    if (key !== "dash-live" && key !== "dash-chart") return;
    delete carried[key];
    var elt = event.detail.elt;
    if (key === "dash-live" && isRefresh(elt) && elt.getAttribute("data-dash-requested") !== here()) {
      event.detail.shouldSwap = false;
      return;
    }
    // A 204 (unchanged figures) swaps nothing, so there is nothing to carry.
    if (!event.detail.shouldSwap) return;
    carried[key] = capture();
  });

  // Runs after the tooltip IIFE's own afterSettle listener has rebound the new
  // chart, so the synthetic move lands on a live handler.
  document.addEventListener("htmx:afterSettle", function (event) {
    var target = event.detail && event.detail.target;
    var key = target && target.id;
    var state = key ? carried[key] : null;
    if (!state) return;
    delete carried[key];
    var pick = document.getElementById("chart-pick");
    state.scrolls.forEach(function (entry) {
      var node = entry[0].isConnected ? entry[0] : pick && entry[2] ? pick.querySelector(entry[2]) : null;
      if (node) node.scrollTop = entry[1];
    });
    var focus = state.focus;
    if (focus && !focus.isConnected) {
      focus = pick && state.option ? optionByKey(pick, state.option) : null;
    }
    if (focus && document.activeElement !== focus) {
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

  document.addEventListener("htmx:beforeHistorySave", function () {
    var pick = document.getElementById("chart-pick");
    if (pick) pick.removeAttribute("hx-preserve");
  });

  document.addEventListener("visibilitychange", function () {
    if (document.hidden || !window.htmx) return;
    var live = document.getElementById("dash-live");
    if (!isRefresh(live) || !window.hfsDashCanRefresh()) return;
    // Targeting the element itself sends HX-Target: dash-live; the request
    // goes through htmx:configRequest above like any tick.
    window.htmx.ajax("GET", here(), {
      source: live,
      target: live,
      select: "#dash-live",
      swap: "outerHTML",
    });
  });
})();

/* Type-picker filter: typeahead over the option rows, same pattern as the
   resource rail's filter. Without JavaScript the list simply scrolls. The
   input is kept across swaps (hx-preserve on it, or on the open picker around
   it, #1078), so its text outlives a refresh or picker swap
   that brings new option rows — and a history restore that brings back old
   ones; the current text is re-applied to them after each. */
(function () {
  "use strict";

  function apply(filter) {
    var panel = filter.closest(".menu__panel");
    if (!panel) return;
    var needle = filter.value.trim().toLowerCase();
    panel.querySelectorAll("[data-pick-name]").forEach(function (option) {
      option.hidden = !!needle && option.dataset.pickName.toLowerCase().indexOf(needle) === -1;
    });
  }

  function applyAll() {
    document.querySelectorAll("[data-pick-filter]").forEach(apply);
  }

  document.addEventListener("input", function (event) {
    var filter = event.target.closest ? event.target.closest("[data-pick-filter]") : null;
    if (filter) apply(filter);
  });
  document.addEventListener("htmx:afterSettle", applyAll);
  document.addEventListener("htmx:historyRestore", applyAll);
})();
