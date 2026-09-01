/*
 * The shared busy/working states (#679): one convention for "this control is
 * doing something". Buttons get `disabled` plus `aria-busy="true"` — the CSS
 * ring in app.css keys off the ARIA attribute, so a script cannot get the
 * visuals without the semantics. Regions are pre-rendered `.busy-status`
 * `role="status"` elements the helper reveals and labels; revealing FIRST
 * and writing the label a tick later is what makes the announcement
 * reliable (a hidden live region is not in the accessibility tree, and text
 * set before it enters is routinely skipped).
 *
 * Fetch-driven scripts call this; htmx controls use `hx-disabled-elt`
 * instead (#581). `window.hfsBusy` is the crate's one exported global: this
 * file loads from the layout, so page scripts (all `defer`, document order)
 * can rely on it. batch.js's renderGeneration guards the same two-quick-picks
 * race for its own rendering; the region generation here is the helper's
 * own so it stands alone — don't wire a third counter.
 */
(function () {
  "use strict";

  /* Per-element call generation: a stale done() from a superseded region()
     call must not clear the newer state. */
  var generations = new WeakMap();

  /* Reveal the region, then write `label` into its [data-busy-label]. */
  function region(el, label) {
    var generation = (generations.get(el) || 0) + 1;
    generations.set(el, generation);
    var labelEl = el.querySelector("[data-busy-label]");
    el.hidden = false;
    setTimeout(function () {
      if (generations.get(el) === generation && labelEl) labelEl.textContent = label;
    }, 0);
    return {
      done: function () {
        if (generations.get(el) !== generation) return;
        /* Invalidate this call's own pending label write too: a clear()
           riding a microtask beats the label's macrotask timer, and the
           stale write would relabel the hidden region. */
        generations.set(el, generation + 1);
        /* Hide first: clearing the text of a visible polite region would
           queue an announcement of nothing. */
        el.hidden = true;
        if (labelEl) labelEl.textContent = "";
      },
    };
  }

  /*
   * Run `work` — a FUNCTION returning a promise, never a promise: the
   * re-entrancy guard must run before the request exists — with `buttons`
   * in the busy state and `opts.alsoDisable` merely disabled. Clears on
   * settle, restoring each control's prior disabled state, and hands focus
   * back to the trigger if disabling ejected it to <body>. `opts.region` /
   * `opts.label` tie a status region to the same lifetime. A work() that
   * navigates away should return a promise that never settles, so its
   * controls stay inert until the page unloads.
   */
  function during(buttons, work, opts) {
    opts = opts || {};
    var held = buttons.some(function (b) {
      return b.getAttribute("aria-busy") === "true";
    });
    if (held) return null;

    /* Read the trigger before the disable loop ejects focus from it. */
    var trigger = document.activeElement;
    var all = buttons.concat(opts.alsoDisable || []);
    var prior = all.map(function (b) {
      return b.disabled;
    });
    buttons.forEach(function (b) {
      b.setAttribute("aria-busy", "true");
    });
    all.forEach(function (b) {
      b.disabled = true;
    });
    var status = opts.region ? region(opts.region, opts.label || "") : null;

    var promise;
    try {
      promise = Promise.resolve(work());
    } catch (e) {
      promise = Promise.reject(e);
    }

    function clear() {
      buttons.forEach(function (b) {
        b.removeAttribute("aria-busy");
      });
      all.forEach(function (b, i) {
        b.disabled = prior[i];
      });
      if (status) status.done();
      if (
        document.activeElement === document.body &&
        trigger &&
        trigger !== document.body &&
        trigger.isConnected &&
        !trigger.disabled
      ) {
        trigger.focus();
      }
    }
    promise.then(clear, clear);
    return promise;
  }

  window.hfsBusy = { during: during, region: region };
})();
