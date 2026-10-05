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
 * Fetch-driven scripts call this. Two document-level rules need no call
 * (#1750). Any `method="post"` form that navigates is marked busy on submit
 * (`aria-busy` on the submitter, every submit button disabled) and a second
 * submit of that form is dropped while the first is in flight; any htmx
 * `<button>` whose request is not a GET is busy for the request's lifetime
 * and ignores repeat clicks. Non-button htmx elements keep using
 * `hx-disabled-elt` (#581). The form's buttons are disabled a tick AFTER the
 * submit event, never inside it: a disabled submitter is left out of the
 * form's entry list, so `name=action value=duplicate` would never be sent.
 * `window.hfsBusy` is the crate's one exported global: this
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

  /* ---- Write forms (#1750) ------------------------------------------- */

  /* Forms whose submit is in flight; `marked` is the iterable twin used to
     restore them. */
  var inFlight = new WeakSet();
  var marked = [];
  var unloadHooked = false;

  function submitButtons(form) {
    return Array.prototype.filter.call(form.elements, function (el) {
      return (
        (el.tagName === "BUTTON" && el.type === "submit") ||
        (el.tagName === "INPUT" && (el.type === "submit" || el.type === "image"))
      );
    });
  }

  /* Leave every marked form as it was: no aria-busy, each button back to its
     prior disabled state. Reached from a bfcache restore and from a
     navigation the native unsaved-changes prompt cancelled. */
  function clearSubmits() {
    var entries = marked;
    marked = [];
    entries.forEach(function (entry) {
      entry.cleared = true;
      inFlight.delete(entry.form);
      if (entry.submitter) entry.submitter.removeAttribute("aria-busy");
      (entry.prior || []).forEach(function (p) {
        p.el.disabled = p.disabled;
      });
    });
  }

  /* Guard (capture): a submit of a form already in flight goes nowhere. */
  document.addEventListener(
    "submit",
    function (event) {
      if (event.target instanceof HTMLFormElement && inFlight.has(event.target)) {
        event.preventDefault();
        event.stopPropagation();
      }
    },
    true,
  );

  /* Mark (bubble): runs after the form's own listeners, so a submit another
     script cancelled (a confirmation, a validation) marks nothing. */
  document.addEventListener("submit", function (event) {
    if (event.defaultPrevented) return;
    var form = event.target;
    if (!(form instanceof HTMLFormElement) || form.method !== "post") return;
    if (form.target && form.target !== "_self") return;

    var entry = { form: form, submitter: event.submitter || null, prior: null, cleared: false };
    inFlight.add(form);
    marked.push(entry);
    if (entry.submitter) entry.submitter.setAttribute("aria-busy", "true");
    /* A later tick: see the header — the submitter must still be enabled
       while the browser builds the entry list. */
    setTimeout(function () {
      if (entry.cleared) return;
      entry.prior = submitButtons(form).map(function (el) {
        var p = { el: el, disabled: el.disabled };
        el.disabled = true;
        return p;
      });
    }, 0);

    if (!unloadHooked) {
      unloadHooked = true;
      /* Registered on the first mark, so it runs after unsaved.js's prompt
         listener and can see whether that prompt cancelled the navigation.
         If the user leaves anyway the buttons re-arm for the rest of that
         navigation. */
      window.addEventListener("beforeunload", function (e) {
        if (e.defaultPrevented) setTimeout(clearSubmits, 0);
      });
    }
  });

  window.addEventListener("pageshow", function (event) {
    if (event.persisted) clearSubmits();
  });

  /* ---- htmx write buttons (#1750) ------------------------------------ */

  var htmxPrior = new WeakMap();

  document.addEventListener("htmx:beforeRequest", function (event) {
    var btn = event.detail && event.detail.elt;
    var config = event.detail && event.detail.requestConfig;
    if (!btn || btn.tagName !== "BUTTON" || !config || config.verb === "get") return;
    if (btn.getAttribute("aria-busy") === "true") {
      event.preventDefault();
      return;
    }
    htmxPrior.set(btn, btn.disabled);
    btn.setAttribute("aria-busy", "true");
    btn.disabled = true;
  });

  document.addEventListener("htmx:afterRequest", function (event) {
    var btn = event.detail && event.detail.elt;
    if (!btn || btn.tagName !== "BUTTON" || !htmxPrior.has(btn)) return;
    var prior = htmxPrior.get(btn);
    htmxPrior.delete(btn);
    if (!btn.isConnected) return;
    btn.removeAttribute("aria-busy");
    btn.disabled = prior;
  });

  window.hfsBusy = { during: during, region: region };
})();
