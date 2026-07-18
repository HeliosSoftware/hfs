// Collapsible primary navigation for the HFS web UI (design: Figma "Search
// V1.0" collapsed rail).
//
// Loaded from <head> WITHOUT `defer`, deliberately: the collapse state must be
// set on <html> before first paint or an expanded sidebar flashes to collapsed.
//
// The choice roams across devices via the per-user settings document
// (`/_user/settings`, the same opaque JSON store the theme uses); localStorage
// stays as a fast-path cache so first paint never waits on the network.
// Precedence: server setting -> localStorage cache -> expanded default. When the
// settings endpoint is unavailable, behavior degrades to localStorage-only.
//
// The toggle button carries `data-toggle-nav`; a single delegated listener keeps
// behavior in one pinned asset (see README.md: no inline script blobs). Narrow
// viewports are forced collapsed by CSS regardless of this preference.
(function () {
  var KEY = "hfs-nav";
  var SETTINGS = "/_user/settings";

  function valid(state) {
    return state === "collapsed" || state === "expanded" ? state : null;
  }

  function apply(state) {
    document.documentElement.setAttribute("data-nav", state);
    // Keep the toggle's ARIA state in sync (the button reflects the *expanded*
    // status of the navigation it controls).
    var buttons = document.querySelectorAll("[data-toggle-nav]");
    for (var i = 0; i < buttons.length; i++) {
      buttons[i].setAttribute("aria-expanded", state === "expanded" ? "true" : "false");
    }
  }

  function cache(state) {
    try {
      localStorage.setItem(KEY, state);
    } catch (e) {
      /* non-fatal: the choice just won't persist on this device */
    }
  }

  // 1. First paint: cache -> expanded default (synchronous, before render).
  var cached = null;
  try {
    cached = valid(localStorage.getItem(KEY));
  } catch (e) {
    /* storage may be unavailable (e.g. blocked); fall through */
  }
  document.documentElement.setAttribute("data-nav", cached || "expanded");

  // Sync the toggle's aria-expanded once the DOM is ready (the button doesn't
  // exist yet at this point in <head>).
  document.addEventListener("DOMContentLoaded", function () {
    apply(document.documentElement.getAttribute("data-nav") || "expanded");
  });

  // 2. Reconcile with the server-side settings document: a roamed choice from
  //    another device wins over the local cache.
  if (window.fetch) {
    fetch(SETTINGS, { headers: { Accept: "application/json" }, credentials: "same-origin" })
      .then(function (response) {
        return response.ok ? response.json() : null;
      })
      .then(function (doc) {
        var server = doc && valid(doc.nav);
        if (!server) return;
        if (server !== document.documentElement.getAttribute("data-nav")) apply(server);
        cache(server);
      })
      .catch(function () {
        /* settings unavailable: keep behaving like the cache-only model */
      });
  }

  // 3. Toggle: flip, apply immediately, cache locally, persist to the settings
  //    document with an RFC 7386 merge-patch so only the nav key changes.
  document.addEventListener("click", function (event) {
    var button = event.target.closest && event.target.closest("[data-toggle-nav]");
    if (!button) return;
    var current = document.documentElement.getAttribute("data-nav") === "collapsed"
      ? "collapsed"
      : "expanded";
    var next = current === "collapsed" ? "expanded" : "collapsed";
    apply(next);
    cache(next);
    if (window.fetch) {
      fetch(SETTINGS, {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        credentials: "same-origin",
        body: JSON.stringify({ nav: next }),
      }).catch(function () {
        /* non-fatal: the local cache still holds the choice */
      });
    }
  });
})();
