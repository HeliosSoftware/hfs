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
 * Shared across every tracker on the page:
 *   - a pill (`.tag.tag--unsaved`) prepended into a tracker's own `cue`;
 *   - one `beforeunload` listener on `window`, guarding real navigation;
 *   - delegated internal links and GET forms ask through `HfsConfirm`
 *     before abandoning a draft, preserving its loaded baseline;
 *   - `confirmDiscard(scope)`, a translated in-page confirmation
 *     (`confirm.js`, #1667) for the in-page closes that do not navigate at
 *     all (a modal, an `addbox` disclosure) — called by the closer itself,
 *     never wired here. It answers with a Promise of a boolean, so the
 *     closer finishes its close in `.then`.
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

  // Boosted body swaps execute scripts again but retain this document's
  // listeners and connected trackers. Reuse the shared guard.
  if (root && root.HfsUnsaved) return;
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

  /* Guided primitives settle on blur, before their value reaches the JSON
   * document. Disabled fields still carry pending values while a structural
   * mutation locks the projection (#1745). Normalize the snapshot before
   * appending pending state so harmless JSON indentation stays equivalent.
   * `container` is the mutation host (body/grid), not necessarily the root
   * that receives input events or the form whose fields are serialized. */
  function withPending(snapshot, container) {
    var value = normalize(snapshot);
    if (!container) return value;
    var pending = "";
    var fields = container.querySelectorAll("[data-set]");
    for (var i = 0; i < fields.length; i++) {
      var el = fields[i];
      var path = el.dataset.set;
      if (!path) continue;
      var original = undefined;
      if (el.tagName === "SELECT") {
        for (var j = 0; j < el.options.length; j++) {
          if (el.options[j].defaultSelected) {
            original = el.options[j].value;
            break;
          }
        }
        if (original === undefined) original = el.options.length ? el.options[0].value : "";
      } else if ("defaultValue" in el) original = el.defaultValue;
      else continue;
      // A primitive's text is FHIR content, even when it resembles JSON.
      // Encode it exactly so whitespace and newlines also distinguish a
      // later edit from a previously acknowledged discard.
      if (el.value !== original) {
        pending += path + "=" + JSON.stringify(el.value) + "\n";
      }
    }
    var picker = typeof window !== "undefined" ? window.HfsEditorAdd : null;
    if (picker && picker.hasPendingMutations && picker.hasPendingMutations(container)) pending += "--mutation--\n";
    return value + (pending ? "\n--pending--\n" + pending : "");
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
  var navigationQuestion = null;
  var replay = null;
  var nativePermit = null;
  var editRevision = 0;
  var lifecycleGeneration = 0;

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
      return withinScope(scope, tracker.root) && dirtyTracker(tracker);
    });
  }

  function dirtyTracker(tracker) {
    return tracker.checkOnExit || tracker.pendingCheck() ? tracker.check() : tracker.isDirty();
  }

  /* The shared in-page confirmation (`window.HfsConfirm`, #1667) with the
   * translated copy on `<body data-msg-unsaved-discard>` — never a hardcoded
   * English fallback (the same degrade-to-nothing contract `vd-editor.js`'s
   * own `saveConfirmMessage` follows): a page whose layout somehow lacks the
   * attribute lets the close through unasked rather than showing the wrong
   * language. Resolves `true` when the close may go ahead. Accepting marks
   * every dirty tracker in scope clean, so the caller's own reset (e.g.
   * `addbox.js` resetting the form it is about to close) never re-triggers
   * this. */
  function confirmDiscard(scope) {
    var dirtyTrackers = trackers.filter(function (tracker) {
      return withinScope(scope, tracker.root) && dirtyTracker(tracker);
    });
    if (dirtyTrackers.length === 0) return Promise.resolve(true);
    var message =
      document.body && document.body.dataset ? document.body.dataset.msgUnsavedDiscard : "";
    if (!message) return Promise.resolve(true);
    var asked = window.HfsConfirm
      ? window.HfsConfirm.ask(message)
      : Promise.resolve(window.confirm(message));
    return asked.then(function (confirmed) {
      if (!confirmed) return false;
      dirtyTrackers.forEach(function (tracker) {
        tracker.markClean();
      });
      return true;
    });
  }

  function suspend() {
    suspended = true;
  }

  /* Navigation never marks a tracker clean. A replay permission lasts only
   * for its own activation; a separate permission is consumed by the native
   * beforeunload check (including responses that keep this document, such as
   * 204). HTMX swaps do not need or receive a native permission. */
  function navigationCopy() {
    var data = document.body && document.body.dataset ? document.body.dataset : {};
    return { message: data.msgUnsavedLeave || "", label: data.msgUnsavedLeaveAction || "" };
  }

  function attribute(elt, name) {
    return elt && elt.getAttribute ? elt.getAttribute(name) : null;
  }

  function currentTarget(target) {
    var base = document.querySelector && document.querySelector("base[target]");
    target = target || attribute(base, "target") || "_self";
    return target === "_self" || target === "_top" || target === "_parent" || (window.name && target === window.name);
  }

  function formAttribute(form, submitter, name, fallback) {
    var override = attribute(submitter, "form" + name);
    return override !== null ? override : attribute(form, name) || fallback;
  }

  function internalUrl(value) {
    try {
      var url = new URL(value, document.baseURI || window.location.href);
      return /^https?:$/.test(url.protocol) && url.origin === window.location.origin ? url : null;
    } catch (invalidUrl) { return null; }
  }

  function linkAction(link) {
    if (!link || !link.isConnected || attribute(link, "href") === null ||
        attribute(link, "download") !== null || !currentTarget(attribute(link, "target"))) return null;
    var href = attribute(link, "href");
    var url = internalUrl(href);
    if (!url) return null;
    var current = new URL(window.location.href);
    // Any fragment in this same document preserves the editor; an identical
    // URL without a fragment can reload and must still be guarded.
    if (url.pathname === current.pathname && url.search === current.search && href.indexOf("#") !== -1) return null;
    return { elt: link, signature: url.href + "\n" + (attribute(link, "target") || ""), kind: "link" };
  }

  function formAction(form, submitter) {
    if (!form || !form.isConnected || (submitter && (!submitter.isConnected || submitter.form !== form))) return null;
    var method = (formAttribute(form, submitter, "method", "get") || "get").toLowerCase();
    var target = formAttribute(form, submitter, "target", "");
    // Native forms treat an invalid method value as GET too.
    if (method === "post" || method === "dialog" || !currentTarget(target)) return null;
    var url = internalUrl(formAttribute(form, submitter, "action", window.location.href) || window.location.href);
    if (!url) return null;
    return { elt: form, submitter: submitter, kind: "form",
      signature: url.href + "\n" + target + "\n" + serialize(form) + "\n" +
        (submitter ? submitter.name + "=" + submitter.value : "") };
  }

  function stillMatches(action) {
    var current = action.kind === "link" ? linkAction(action.elt) : formAction(action.elt, action.submitter);
    return current && current.signature === action.signature;
  }

  function snapshots() {
    return trackers.filter(function (tracker) { return withinScope(null, tracker.root); }).map(function (tracker) {
      var value = tracker.snapshot();
      return { tracker: tracker, value: value, pending: value.indexOf("\n--pending--\n") !== -1 };
    });
  }

  function changedSince(question) {
    return question.snapshots.some(function (snapshot) {
      if (!snapshot.tracker.root.isConnected) return true;
      var changed = snapshot.value !== snapshot.tracker.snapshot();
      // A primitive typed before blur may settle into JSON while the dialog
      // is open. That is the same edit, not a second decision. Genuine input
      // in the editor (including CodeMirror's native input) revokes it.
      return changed && (!snapshot.pending || question.revision !== editRevision);
    });
  }

  function askNavigation(action, resume) {
    if (navigationQuestion) return;
    var copy = navigationCopy();
    if (!copy.message || !copy.label) return;
    var question = { action: action, snapshots: snapshots(), revision: editRevision, generation: lifecycleGeneration };
    navigationQuestion = question;
    var trigger = action.submitter || action.elt;
    if (trigger.focus) trigger.focus({ preventScroll: true });
    // Moving focus can emit the primitive's native change/blur and start
    // its already-authored mutation. It belongs to the snapshot just asked.
    question.revision = editRevision;
    var asked = window.HfsConfirm
      ? window.HfsConfirm.ask(copy.message, { confirmLabel: copy.label })
      : Promise.resolve(window.confirm(copy.message));
    asked.then(function (confirmed) {
      if (navigationQuestion !== question) return;
      navigationQuestion = null;
      if (!confirmed || !stillMatches(action)) return;
      if (changedSince(question)) {
        askNavigation(action, resume);
        return;
      }
      resume(action, question);
    }, function () {
      if (navigationQuestion === question) navigationQuestion = null;
    });
  }

  function replayNative(action) {
    replay = { action: action, event: null };
    try {
      if (action.kind === "link") action.elt.click();
      else HTMLFormElement.prototype.requestSubmit.call(action.elt, action.submitter || undefined);
    } finally {
      // requestSubmit can produce no submit event (failed validation); a
      // later listener may cancel the replay after our delegated listener.
      if (!replay.event || replay.event.defaultPrevented || !stillMatches(action)) nativePermit = null;
      replay = null;
    }
  }

  function guardNative(event, action) {
    if (!action) return;
    if (replay && replay.action.elt === action.elt) {
      replay.event = event;
      if (action.signature !== replay.action.signature || event.defaultPrevented) {
        event.preventDefault();
        return;
      }
      nativePermit = action;
      return;
    }
    nativePermit = null;
    if (event.defaultPrevented || !isDirty()) return;
    var copy = navigationCopy();
    // Native navigation remains guarded by beforeunload when copy is absent.
    if (!copy.message || !copy.label) return;
    event.preventDefault();
    askNavigation(action, replayNative);
  }

  function guardHtmx(event) {
    var detail = event.detail || {};
    if (event.defaultPrevented || String(detail.verb).toLowerCase() !== "get" || !detail.elt) return;
    var elt = detail.elt;
    var swapOwner = elt.closest && elt.closest("[hx-swap], [data-hx-swap]");
    var swap = attribute(swapOwner, "hx-swap") || attribute(swapOwner, "data-hx-swap") || "";
    if (/^(none|beforebegin|afterbegin|beforeend|afterend)(\s|$)/.test(swap)) return;
    var triggering = detail.triggeringEvent;
    if (triggering && triggering.type === "click" &&
        (triggering.button !== 0 || triggering.ctrlKey || triggering.metaKey || triggering.shiftKey || triggering.altKey)) return;
    var action = elt.tagName === "A" ? linkAction(elt)
      : elt.tagName === "FORM" ? formAction(elt, triggering && triggering.submitter) : null;
    if (!action || !detail.target) return;
    var abandons = trackers.some(function (tracker) {
      return withinScope(detail.target, tracker.root) && dirtyTracker(tracker);
    });
    if (!abandons) return;
    // This asynchronous request never unloads the document. If translated
    // copy is missing, leave the draft in place rather than silently swap.
    event.preventDefault();
    nativePermit = null;
    askNavigation(action, function (approvedAction, authorization) {
      if (!detail.target || !detail.target.isConnected) return;
      function issueAuthorized(decision) {
        // pageshow also revokes authorizations held by the distinct action
        // dialog, after the first discard question has already closed.
        if (decision.generation !== lifecycleGeneration) return;
        if (!detail.target.isConnected || !stillMatches(approvedAction)) return;
        if (changedSince(decision)) {
          // The action confirmation is already answered, but that answer
          // cannot discard a newer draft. Carry a fresh discard decision to
          // this same final boundary rather than silently issuing the GET.
          askNavigation(approvedAction, function (nextAction, nextDecision) {
            issueAuthorized(nextDecision);
          });
          return;
        }
        detail.issueRequest(true);
      }
      // A distinct hx-confirm question must still be answered, if present.
      var proceed = detail.question
        ? (window.HfsConfirm ? window.HfsConfirm.ask(detail.question) : Promise.resolve(window.confirm(detail.question)))
        : Promise.resolve(true);
      proceed.then(function (confirmed) { if (confirmed) issueAuthorized(authorization); });
    });
  }

  function registerGlobalListeners() {
    if (globalListenersRegistered) return;
    globalListenersRegistered = true;

    window.addEventListener("beforeunload", function (event) {
      if (nativePermit) {
        var allowed = stillMatches(nativePermit);
        nativePermit = null;
        if (allowed) return;
      }
      if (suspended) return;
      if (!isDirty()) return;
      event.preventDefault();
      event.returnValue = "";
    });

    /* A page restored from the back/forward cache is a fresh navigation as
     * far as this guard is concerned — re-arm it. */
    window.addEventListener("pageshow", function () {
      lifecycleGeneration++;
      suspended = false;
      nativePermit = null;
      replay = null;
      navigationQuestion = null;
    });

    document.addEventListener("htmx:confirm", guardHtmx, true);
    document.addEventListener("click", function (event) {
      if (event.button !== 0 || event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
      var link = event.target && event.target.closest ? event.target.closest("a[href]") : null;
      guardNative(event, linkAction(link));
    });

    // Synthetic textarea input also represents the guided form settling.
    // Listen for native input in the actual editor, not for its JSON echo.
    ["input", "change"].forEach(function (type) {
      document.addEventListener(type, function (event) {
        if (!event.isTrusted) return;
        var edited = trackers.some(function (tracker) {
          return withinScope(null, tracker.root) && ((tracker.root.contains && tracker.root.contains(event.target)) ||
            (tracker.form && event.target.form === tracker.form));
        });
        if (edited) { editRevision++; nativePermit = null; }
      }, true);
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
        var action = formAction(event.target, event.submitter);
        if (action) { guardNative(event, action); return; }
        var submittedTrackedForm = trackers.some(function (tracker) {
          return tracker.form && tracker.form === event.target;
        });
        var method = (formAttribute(event.target, event.submitter, "method", "get") || "get").toLowerCase();
        if (submittedTrackedForm && method === "post") suspend();
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
    // A failed native Save re-renders the submitted draft as the initial
    // document. That text was not persisted and must not become clean just
    // because the tracker has been mounted again on the error response.
    var unsavedDraft = attribute(form, "data-unsaved-draft") !== null;
    var discarded = null;
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
      var current = normalize(read());
      // markClean acknowledges this exact snapshot without rewriting the
      // loaded baseline. A fresh exit check must not immediately ask again;
      // a subsequent edit resumes comparison against the original baseline.
      if (discarded !== null && current !== discarded) discarded = null;
      dirty = (unsavedDraft || current !== baseline) && current !== discarded;
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
      unsavedDraft = false;
      discarded = null;
      dirty = false;
      updatePill();
    }

    function markClean() {
      discarded = normalize(read());
      dirty = false;
      updatePill();
    }

    trackedRoot.addEventListener("input", scheduleCheck);
    trackedRoot.addEventListener("change", scheduleCheck);
    trackedRoot.addEventListener("htmx:afterSwap", scheduleCheck);
    trackedRoot.addEventListener("htmx:afterSettle", scheduleCheck);
    trackedRoot.addEventListener("hfs:editor-mutation", scheduleCheck);
    if (form) {
      /* Deferred: a native reset restores control values AFTER the `reset`
       * event itself finishes dispatching. */
      form.addEventListener("reset", function () {
        setTimeout(check, 0);
      });
    }

    var tracker = {
      // Editors that expose pending fields opt into synchronous exit checks;
      // other hosts keep their existing event-driven loading lifecycle.
      checkOnExit: options.checkOnExit === true,
      root: trackedRoot,
      form: form,
      check: check,
      reset: reset,
      markClean: markClean,
      snapshot: function () { return normalize(read()); },
      pendingCheck: function () { return rafHandle !== null; },
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
    withPending: withPending,
    serialize: serialize,
    track: track,
    isDirty: isDirty,
    confirmDiscard: confirmDiscard,
    suspend: suspend,
  };
});
