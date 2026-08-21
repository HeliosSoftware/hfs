/* Dashboard chart tooltip (#555): a hover readout over the server-rendered
   SVG. The inert #chart-data carrier holds the bucket labels and every plotted
   series' values with their SVG coordinates — the server did the chart math,
   this script only maps the pointer to the nearest bucket and shows what is
   there. Without JavaScript the chart, its legend, and the tabular alternative
   are complete; this only adds the readout. init() is re-run whenever the
   view-all toggle (#599) swaps in a fresh chart card — the swap brings new
   #chart-wrap/#chart-tip/#chart-data nodes, so the listeners bound below need
   rebinding to them; the previous nodes are detached, so there is no
   duplicate-listener risk. */
(function () {
  "use strict";

  function init() {
    var wrap = document.getElementById("chart-wrap");
    var tip = document.getElementById("chart-tip");
    var carrier = document.getElementById("chart-data");
    if (!wrap || !tip || !carrier) return;

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
        document.dispatchEvent(new CustomEvent("hfs:chart-swapped"));
      })
      .catch(function () {
        window.location = href;
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
