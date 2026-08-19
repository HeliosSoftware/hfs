/* Dashboard chart tooltip (#555): a hover readout over the server-rendered
   SVG. The inert #chart-data carrier holds the bucket labels and every plotted
   series' values with their SVG coordinates — the server did the chart math,
   this script only maps the pointer to the nearest bucket and shows what is
   there. Without JavaScript the chart, its legend, and the tabular alternative
   are complete; this only adds the readout. */
(function () {
  "use strict";

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
})();
