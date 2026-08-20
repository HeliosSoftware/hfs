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

  var MAX_RECENT = 3;
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
