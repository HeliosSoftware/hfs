/*
 * Shared unsaved-changes tracker (#1240): one call per editing screen, no
 * storage anywhere. `window.HfsUnsaved.track({ root, form?, read?, cue? })`
 * keeps a single dirty flag for a form (or any caller-supplied `read()`)
 * computed against the state it loaded with — every value `trim()`med, and
 * any value that parses as JSON compared by its canonical form
 * (`JSON.stringify(JSON.parse(v))`, the same comparison `editor-pair.js`
 * already uses for its own JSON<->form sync) rather than letter by letter, so
 * a guided form that reindents JSON on every sync never raises a false
 * "unsaved" flag. Undoing an edit back to the loaded state clears the flag.
 *
 * Three built-in guards, shared across every tracker on the page:
 *   - a pill (`.tag.tag--unsaved`) prepended into a tracker's own `cue`;
 *   - one `beforeunload` listener on `window`, guarding real navigation;
 *   - `confirmDiscard(scope)`, a translated `window.confirm` for the
 *     in-page closes that do not navigate at all (a modal, an `addbox`
 *     disclosure) — called by the closer itself, never wired here.
 *
 * `suspend()` arms a one-shot exception for the guard's own next
 * `beforeunload` check (a delete or another redirect the page triggers
 * itself), and re-arms on `pageshow` so a page restored from the back/forward
 * cache is guarded again. Submitting a tracked form suspends automatically
 * (see the shared `submit` listener below) — the navigation a save itself
 * causes must never ask; htmx submits, failed validation, and confirms that
 * cancel the submit all call `preventDefault`, so they leave the guard armed.
 *
 * UMD wrapper — same shape as `editor-pair.js` — so `e2e/unit/unsaved.test.cjs`
 * can `require()` this file under plain Node with no `window`/`document` at
 * all: nothing above touches either at load time, only inside `track()` and
 * the functions it calls.
 */
(function (root, factory) {
  "use strict";

  var api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.HfsUnsaved = api;
})(typeof window !== "undefined" ? window : null, function () {
  "use strict";

  /* ---- normalize / serialize --------------------------------------------- */

  /* `value` -> a comparable string: trimmed, and — only when the trimmed
   * text looks like JSON (starts with "{" or "[") and actually parses — its
   * canonical `JSON.stringify(JSON.parse(...))` form, so whitespace-only
   * reformatting never counts as a change. Invalid JSON falls back to the
   * trimmed text itself rather than throwing. `null`/`undefined` -> "". */
  function normalize(value) {
    if (value === null || value === undefined) return "";
    var text = String(value).trim();
    var first = text.charAt(0);
    if (first === "{" || first === "[") {
      try {
        return JSON.stringify(JSON.parse(text));
      } catch (invalidJson) {
        return text;
      }
    }
    return text;
  }

  var SKIPPED_TYPES = {
    button: true,
    submit: true,
    reset: true,
    image: true,
    file: true,
  };

  /* `form.elements` itself, robust to a control named `elements` (e.g.
   * bulk-export's own `_elements` filter field): once such a control exists,
   * it shadows `HTMLFormElement.prototype.elements` as an own property, so
   * plain `form.elements` would read the *input*, not the collection. Read
   * the real getter straight off the prototype for an actual form; anything
   * else (a plain Node/ad hoc object, as the unit tests under `require()`
   * pass) falls back to the property as-is. */
  function formElements(form) {
    if (typeof HTMLFormElement !== "undefined" && form instanceof HTMLFormElement) {
      return Object.getOwnPropertyDescriptor(HTMLFormElement.prototype, "elements").get.call(form);
    }
    return form.elements;
  }

  /* A form's own state as one comparable string: one "name=normalize(value)"
   * line per named, non-skipped control in `form.elements` (document order —
   * that collection already includes controls associated by a `form=`
   * attribute elsewhere in the document, not just descendants), robust to a
   * control named `elements` (see `formElements` above). Checkboxes
   * always emit a line, `name=` empty when unchecked, so toggling either way
   * counts as a change; a radio emits its own `value` but only when checked
   * (an unchecked radio in a group carries no information beyond "not this
   * one"), so picking a different option in the same group changes the line
   * that name produces — `name=checked` for every option would not. A
   * `select[multiple]` emits one line per selected option. */
  function serialize(form) {
    var lines = "";
    var elements = formElements(form);
    for (var i = 0; i < elements.length; i++) {
      var el = elements[i];
      var name = el.name;
      if (!name) continue;
      var type = (el.type || "").toLowerCase();
      if (SKIPPED_TYPES[type]) continue;

      if (type === "checkbox") {
        lines += name + "=" + normalize(el.checked ? "checked" : "") + "\n";
        continue;
      }
      if (type === "radio") {
        if (el.checked) lines += name + "=" + normalize(el.value) + "\n";
        continue;
      }
      if (el.tagName === "SELECT" && el.multiple) {
        var options = el.options;
        for (var j = 0; j < options.length; j++) {
          if (options[j].selected) lines += name + "=" + normalize(options[j].value) + "\n";
        }
        continue;
      }
      lines += name + "=" + normalize(el.value) + "\n";
    }
    return lines;
  }

  /* ---- shared state across every tracker on the page --------------------- */

  var trackers = [];
  var suspended = false;
  var globalListenersRegistered = false;

  function withinScope(scope, trackedRoot) {
    /* A root an htmx swap has since replaced is never in scope — otherwise a
     * dirty flag computed against a root no longer in the document would
     * keep `beforeunload` armed with nothing left for the user to save. */
    if (!trackedRoot.isConnected) return false;
    if (!scope) return true;
    if (scope === trackedRoot) return true;
    return !!(scope.contains && scope.contains(trackedRoot));
  }

  function isDirty(scope) {
    return trackers.some(function (tracker) {
      return withinScope(scope, tracker.root) && tracker.isDirty();
    });
  }

  /* `window.confirm` with the translated copy on `<body data-msg-unsaved-
   * discard>` — never a hardcoded English fallback (the same degrade-to-
   * nothing contract `vd-editor.js`'s own `saveConfirmMessage` follows): a
   * page whose layout somehow lacks the attribute lets the close through
   * unasked rather than showing the wrong language. Accepting marks every
   * dirty tracker in scope clean, so the caller's own reset (e.g. `addbox.js`
   * resetting the form it is about to close) never re-triggers this. */
  function confirmDiscard(scope) {
    var dirtyTrackers = trackers.filter(function (tracker) {
      return withinScope(scope, tracker.root) && tracker.isDirty();
    });
    if (dirtyTrackers.length === 0) return true;
    var message =
      document.body && document.body.dataset ? document.body.dataset.msgUnsavedDiscard : "";
    if (!message) return true;
    if (!window.confirm(message)) return false;
    dirtyTrackers.forEach(function (tracker) {
      tracker.markClean();
    });
    return true;
  }

  function suspend() {
    suspended = true;
  }

  function registerGlobalListeners() {
    if (globalListenersRegistered) return;
    globalListenersRegistered = true;

    window.addEventListener("beforeunload", function (event) {
      if (suspended) return;
      if (!isDirty()) return;
      event.preventDefault();
      event.returnValue = "";
    });

    /* A page restored from the back/forward cache is a fresh navigation as
     * far as this guard is concerned — re-arm it. */
    window.addEventListener("pageshow", function () {
      suspended = false;
    });

    /* The navigation a tracked form's own submit causes must not also ask —
     * `suspend()` here, not inside `track()`, so it covers every tracker
     * sharing this one document-level listener. A submit htmx intercepts, a
     * failed validation, or a confirm that itself calls `preventDefault`
     * never reaches this: `event.defaultPrevented` is checked first. */
    document.addEventListener(
      "submit",
      function (event) {
        if (event.defaultPrevented) return;
        var submittedTrackedForm = trackers.some(function (tracker) {
          return tracker.form && tracker.form === event.target;
        });
        if (submittedTrackedForm) suspend();
      },
      false
    );
  }

  /* ---- track --------------------------------------------------------------- */

  function track(options) {
    options = options || {};
    var trackedRoot = options.root;
    var form = options.form || (trackedRoot && trackedRoot.tagName === "FORM" ? trackedRoot : null);
    var read = options.read || (form ? function () { return serialize(form); } : null);
    if (!read) throw new Error("HfsUnsaved.track: form or read required");
    var cue = options.cue || null;

    registerGlobalListeners();

    var baseline = normalize(read());
    var dirty = false;
    var pill = null;
    var rafHandle = null;

    function ensurePill() {
      if (pill || !cue) return pill;
      var text =
        document.body && document.body.dataset ? document.body.dataset.msgUnsaved : "";
      if (!text) return null;
      pill = document.createElement("span");
      pill.className = "tag tag--unsaved";
      pill.setAttribute("role", "status");
      pill.hidden = true;
      pill.textContent = text;
      cue.prepend(pill);
      return pill;
    }

    function updatePill() {
      var el = ensurePill();
      if (el) el.hidden = !dirty;
    }

    function check() {
      dirty = normalize(read()) !== baseline;
      updatePill();
      return dirty;
    }

    /* Coalesced with requestAnimationFrame: several `input`/`change`
     * events in the same tick (a combobox's own chip rebuild, a form-driven
     * editor-pair.js sync) recompute once, not once per event. */
    function scheduleCheck() {
      if (rafHandle !== null) return;
      rafHandle = window.requestAnimationFrame(function () {
        rafHandle = null;
        check();
      });
    }

    function reset() {
      baseline = normalize(read());
      dirty = false;
      updatePill();
    }

    function markClean() {
      dirty = false;
      updatePill();
    }

    trackedRoot.addEventListener("input", scheduleCheck);
    trackedRoot.addEventListener("change", scheduleCheck);
    trackedRoot.addEventListener("htmx:afterSwap", scheduleCheck);
    trackedRoot.addEventListener("htmx:afterSettle", scheduleCheck);
    if (form) {
      /* Deferred: a native reset restores control values AFTER the `reset`
       * event itself finishes dispatching. */
      form.addEventListener("reset", function () {
        setTimeout(check, 0);
      });
    }

    var tracker = {
      root: trackedRoot,
      form: form,
      check: check,
      reset: reset,
      markClean: markClean,
      isDirty: function () {
        return dirty;
      },
    };
    trackers.push(tracker);
    check();
    return tracker;
  }

  return {
    normalize: normalize,
    serialize: serialize,
    track: track,
    isDirty: isDirty,
    confirmDiscard: confirmDiscard,
    suspend: suspend,
  };
});
