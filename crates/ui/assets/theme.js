// Theme selection for the HFS web UI (design: Figma "Dashboard V1.1").
//
// Loaded from <head> WITHOUT `defer`, deliberately: the theme attribute must
// be set before first paint or a stored dark preference flashes light.
//
// The choice roams across devices via the per-user settings document
// (`/_user/settings`, an opaque JSON doc from #151); localStorage stays as a
// fast-path cache so first paint never waits on the network. Precedence:
// server setting -> localStorage cache -> OS preference -> light. When the
// settings endpoint is unavailable (store not configured, offline), behavior
// degrades to the old localStorage-only model.
//
// The toggle buttons in the top bar carry `data-set-theme="light|dark"`; a
// single delegated listener keeps behavior in one pinned asset (see
// README.md: no inline script blobs).
(function () {
  var KEY = "hfs-theme";
  var SETTINGS = "/_user/settings";

  function valid(theme) {
    return theme === "light" || theme === "dark" ? theme : null;
  }

  function apply(theme) {
    document.documentElement.setAttribute("data-theme", theme);
  }

  function cache(theme) {
    try {
      localStorage.setItem(KEY, theme);
    } catch (e) {
      /* non-fatal: theme just won't persist on this device */
    }
  }

  // 1. First paint: cache -> OS -> light (synchronous, before render).
  var cached = null;
  try {
    cached = valid(localStorage.getItem(KEY));
  } catch (e) {
    /* storage may be unavailable (e.g. blocked); fall through */
  }
  apply(
    cached ||
      (window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches
        ? "dark"
        : "light")
  );

  // 2. Reconcile with the server-side settings document (source of truth):
  //    a roamed choice from another device wins over the local cache.
  if (window.fetch) {
    fetch(SETTINGS, { headers: { Accept: "application/json" }, credentials: "same-origin" })
      .then(function (response) {
        return response.ok ? response.json() : null;
      })
      .then(function (doc) {
        var server = doc && valid(doc.theme);
        if (!server) return;
        if (server !== document.documentElement.getAttribute("data-theme")) apply(server);
        cache(server);
      })
      .catch(function () {
        /* settings unavailable: keep behaving like the cache-only model */
      });
  }

  // 3. Toggle: apply immediately, cache locally, persist to the settings
  //    document with an RFC 7386 merge-patch so only the theme key changes.
  document.addEventListener("click", function (event) {
    var button = event.target.closest && event.target.closest("[data-set-theme]");
    if (!button) return;
    var next = valid(button.getAttribute("data-set-theme"));
    if (!next) return;
    apply(next);
    cache(next);
    if (window.fetch) {
      fetch(SETTINGS, {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        credentials: "same-origin",
        body: JSON.stringify({ theme: next }),
      }).catch(function () {
        /* non-fatal: the local cache still holds the choice */
      });
    }
  });
})();
