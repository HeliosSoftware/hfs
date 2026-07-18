/*
 * Resource Filter rail: "Recently used" group (#238).
 *
 * Progressive enhancement only — the rail is fully server-rendered and works
 * without this script; recents are a per-browser convenience kept in
 * localStorage (`data-recent-key` on the group container). The group stays
 * hidden until it has at least one entry, and entries are clones of the
 * server-rendered rail items so hrefs, counts, and i18n come from the page,
 * never from here.
 */
(function () {
  "use strict";

  var MAX_RECENT = 3;

  var group = document.querySelector("[data-recent-key]");
  var list = document.getElementById("sp-rail-list");
  if (!group || !list) return;
  var key = group.getAttribute("data-recent-key");

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
    group.querySelectorAll(".filter-rail__item").forEach(function (n) { n.remove(); });
    var shown = 0;
    readRecents().forEach(function (name) {
      var item = list.querySelector('[data-type="' + CSS.escape(name) + '"]');
      if (!item) return;
      group.appendChild(item.cloneNode(true));
      shown++;
    });
    group.hidden = shown === 0;
  }

  document.addEventListener("click", function (event) {
    var item = event.target.closest("[data-type]");
    if (!item) return;
    var name = item.getAttribute("data-type");
    var recents = readRecents().filter(function (n) { return n !== name; });
    recents.unshift(name);
    writeRecents(recents);
  });

  renderRecents();
})();
