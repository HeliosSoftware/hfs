// Collapsible primary navigation for the HFS web UI (design: Brett's Figma
// frames, where the sidebar can be a narrow icon rail — #282).
//
// Loaded from <head> WITHOUT `defer`, deliberately: the collapse state must be
// set on <html> before first paint, or an expanded sidebar flashes to narrow
// (the same reasoning as theme.js).
//
// The choice is a local UI preference, cached in localStorage. A single
// delegated listener toggles it; the toggle button carries `data-nav-toggle`.
(function () {
  var KEY = "hfs-nav";
  var root = document.documentElement;

  function apply(collapsed) {
    root.classList.toggle("nav-collapsed", !!collapsed);
  }

  // Before first paint: read the cached preference.
  try {
    apply(window.localStorage.getItem(KEY) === "collapsed");
  } catch (e) {
    /* private mode / storage disabled: default expanded */
  }

  // The button lives in the sidebar; wire it once the DOM is ready.
  document.addEventListener("click", function (event) {
    if (!event.target.closest("[data-nav-toggle]")) return;
    var collapsed = !root.classList.contains("nav-collapsed");
    apply(collapsed);
    try {
      window.localStorage.setItem(KEY, collapsed ? "collapsed" : "expanded");
    } catch (e) {
      /* ignore */
    }
  });
})();
