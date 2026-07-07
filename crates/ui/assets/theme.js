// Theme selection for the HFS web UI (design: Figma "Dashboard V1.1").
//
// Loaded from <head> WITHOUT `defer`, deliberately: the theme attribute must
// be set before first paint or a stored dark preference flashes light.
// Order of precedence: stored choice -> OS preference -> light.
//
// The toggle buttons in the top bar carry `data-set-theme="light|dark"`; a
// single delegated listener keeps behavior in one pinned asset (see
// README.md: no inline script blobs).
(function () {
  var stored = null;
  try {
    stored = localStorage.getItem("hfs-theme");
  } catch (e) {
    /* storage may be unavailable (e.g. blocked); fall through */
  }
  var theme =
    stored === "light" || stored === "dark"
      ? stored
      : window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches
        ? "dark"
        : "light";
  document.documentElement.setAttribute("data-theme", theme);

  document.addEventListener("click", function (event) {
    var button = event.target.closest && event.target.closest("[data-set-theme]");
    if (!button) return;
    var next = button.getAttribute("data-set-theme");
    document.documentElement.setAttribute("data-theme", next);
    try {
      localStorage.setItem("hfs-theme", next);
    } catch (e) {
      /* non-fatal: theme just won't persist */
    }
  });
})();
