/*
 * The shared in-page confirmation (#1667): one dialog for every "are you
 * sure?" the UI asks, instead of the browser's own `window.confirm` box
 * ("localhost:18080 says") on some pages and an in-page one on others.
 *
 * `window.HfsConfirm.ask(message, { danger?, confirmLabel? })` returns a
 * Promise of `true` (confirmed) or `false` (cancelled — the Cancel button,
 * Esc, or a click on the backdrop). It is a native `<dialog>` opened with
 * `showModal()`, so focus is trapped inside it, the page behind is inert,
 * and focus goes back to the trigger on close. The button labels are the
 * translated copy on `<body data-msg-confirm-ok data-msg-confirm-cancel>`.
 *
 * One question at a time: an `ask` made while another is still open answers
 * `false` at once rather than stacking a second dialog over the first — the
 * question the user is looking at is the one that counts.
 *
 * A `<details data-confirm>` is the no-JavaScript confirmation of a
 * destructive form (the disclosure holds the warning and the form). With
 * JavaScript its `summary` does not open the disclosure: it asks here with
 * `data-confirm` as the message (`data-confirm-label` as the confirm button
 * text, `data-confirm-danger` for the danger style) and, once confirmed,
 * submits the form the disclosure contains.
 *
 * Clicks and keys inside the dialog stop at the dialog, so the page's own
 * document-level handlers (addbox.js's outside-click and Esc closes,
 * resources.js's Esc) never see them and never ask again on their own.
 *
 * htmx's `hx-confirm` goes through here too: the `htmx:confirm` listener
 * below asks in-page and issues the request only once confirmed. A trigger
 * carrying `data-confirm-danger` gets the danger-styled confirm button.
 *
 * The `beforeunload` prompt (unsaved.js) stays the browser's own: no page
 * may draw its own UI there.
 */
(function () {
  "use strict";

  var open = null;
  var sequence = 0;

  function labels() {
    var data = document.body && document.body.dataset ? document.body.dataset : {};
    return { ok: data.msgConfirmOk || "", cancel: data.msgConfirmCancel || "" };
  }

  function isOpen() {
    return !!open;
  }

  function ask(message, opts) {
    opts = opts || {};
    if (open) return Promise.resolve(false);
    var text = labels();
    /* A browser without <dialog>, or a page whose layout somehow lacks the
       translated labels, still gets asked — natively, in the browser's own
       language — rather than an untranslated in-page dialog or no question
       at all. */
    if (typeof HTMLDialogElement === "undefined" || !text.ok || !text.cancel) {
      return Promise.resolve(window.confirm(message));
    }

    sequence += 1;
    var messageId = "hfs-confirm-message-" + sequence;

    var dialog = document.createElement("dialog");
    dialog.className = "confirm-dialog";
    dialog.setAttribute("aria-describedby", messageId);
    dialog.setAttribute("aria-label", message);

    var body = document.createElement("p");
    body.className = "confirm-dialog__message";
    body.id = messageId;
    body.textContent = message;

    var actions = document.createElement("div");
    actions.className = "confirm-dialog__actions";

    var cancel = document.createElement("button");
    cancel.type = "button";
    cancel.className = "btn";
    cancel.setAttribute("data-confirm-cancel", "");
    cancel.textContent = text.cancel;
    /* The safe answer has focus first: Enter right after the dialog opens
       keeps what the user has rather than destroying it. */
    cancel.autofocus = true;

    var ok = document.createElement("button");
    ok.type = "button";
    ok.className = opts.danger ? "btn btn--danger" : "btn btn--primary";
    ok.setAttribute("data-confirm-ok", "");
    ok.textContent = opts.confirmLabel || text.ok;

    actions.appendChild(cancel);
    actions.appendChild(ok);
    dialog.appendChild(body);
    dialog.appendChild(actions);

    var trigger = document.activeElement;

    return new Promise(function (resolve) {
      var answer = false;

      function finish(value) {
        answer = value;
        if (dialog.open) dialog.close();
      }

      cancel.addEventListener("click", function () {
        finish(false);
      });
      ok.addEventListener("click", function () {
        finish(true);
      });
      dialog.addEventListener("click", function (event) {
        /* A click on the ::backdrop targets the dialog itself. */
        if (event.target === dialog) finish(false);
        event.stopPropagation();
      });
      dialog.addEventListener("keydown", function (event) {
        event.stopPropagation();
      });
      /* Esc: the browser closes the dialog itself; `answer` stays false. */
      dialog.addEventListener("close", function () {
        dialog.remove();
        open = null;
        if (
          trigger &&
          trigger.isConnected &&
          typeof trigger.focus === "function" &&
          (document.activeElement === document.body || !document.activeElement)
        ) {
          trigger.focus();
        }
        resolve(answer);
      });

      open = dialog;
      document.body.appendChild(dialog);
      dialog.showModal();
    });
  }

  document.addEventListener("htmx:confirm", function (event) {
    var detail = event.detail || {};
    if (!detail.question) return;
    event.preventDefault();
    var elt = detail.elt;
    var danger = !!(elt && elt.hasAttribute && elt.hasAttribute("data-confirm-danger"));
    ask(detail.question, { danger: danger }).then(function (confirmed) {
      if (confirmed) detail.issueRequest(true);
    });
  });

  document.addEventListener("click", function (event) {
    var summary = event.target && event.target.closest ? event.target.closest("summary") : null;
    var details = summary ? summary.parentElement : null;
    if (!details || !details.matches("details[data-confirm]")) return;
    var form = details.querySelector("form");
    if (!form) return;
    event.preventDefault();
    ask(details.dataset.confirm, {
      danger: details.hasAttribute("data-confirm-danger"),
      confirmLabel: details.dataset.confirmLabel,
    }).then(function (confirmed) {
      if (!confirmed) return;
      if (typeof form.requestSubmit === "function") form.requestSubmit();
      else form.submit();
    });
  });

  window.HfsConfirm = { ask: ask, isOpen: isOpen };
})();
