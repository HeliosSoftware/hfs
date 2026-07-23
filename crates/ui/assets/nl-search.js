/*
 * Natural-language search (issue #255), on the Search page only.
 *
 * The one thing this script does NOT do is produce results. It posts the
 * user's text to /$nl-search, which returns a *query* — the server executes
 * nothing. The query lands in the editable strip the visual builder already
 * owns (saved-queries.js), so the user reads it, corrects it if the model got
 * something wrong, and runs it with the same explicit Run click as any other
 * search. There is no silent translation and no second search path: auth,
 * tenancy, audit and search semantics all stay where they already are.
 *
 * The endpoint answers `supported: false` for anything that is not a search
 * over this server's data. That is not an error — it is the expected answer to
 * "write me a bubble sort", and it is rendered as a plain refusal.
 */
(function () {
  "use strict";

  var page = document.querySelector(".search-page");
  var pane = document.getElementById("nl-pane");
  var form = document.getElementById("saved-query-form");
  var urlInput = form && form.elements.url;
  if (!page || !pane || !urlInput || !window.fetch) return;

  var text = document.getElementById("nl-text");
  var submit = document.getElementById("nl-submit");
  var answer = document.getElementById("nl-answer");
  var messages = pane.dataset;

  /* ---- Mode toggle: two ways to write one query ----------------------- */

  var modeButtons = page.querySelectorAll("[data-mode-btn]");
  function setMode(mode) {
    page.dataset.mode = mode;
    modeButtons.forEach(function (button) {
      button.setAttribute(
        "aria-pressed",
        button.dataset.modeBtn === mode ? "true" : "false"
      );
    });
    if (mode === "nl" && text) text.focus();
  }
  modeButtons.forEach(function (button) {
    button.addEventListener("click", function () {
      setMode(button.dataset.modeBtn);
    });
  });

  /* ---- Rendering the translation -------------------------------------- */

  function clearAnswer() {
    answer.textContent = "";
    answer.hidden = true;
    answer.className = "nl-answer";
  }

  function note(className, message) {
    var el = document.createElement("p");
    el.className = className;
    el.textContent = message;
    return el;
  }

  /* A refusal, a rate-limit reply, or a failed translation: say so plainly
   * and leave the query strip untouched. */
  function showMessage(className, message) {
    clearAnswer();
    answer.className = "nl-answer " + className;
    answer.appendChild(note("nl-answer__text", message));
    answer.hidden = false;
  }

  /* A translation: the query goes into the strip (editable, not run), the
   * explanation and any caveats go under it. */
  function showTranslation(result) {
    clearAnswer();
    answer.className = "nl-answer nl-answer--ok";

    if (result.explanation)
      answer.appendChild(note("nl-answer__text", result.explanation));

    var caveats = result.caveats || [];
    if (caveats.length) {
      var heading = document.createElement("p");
      heading.className = "nl-answer__caveats-heading";
      heading.textContent = messages.msgCaveats;
      answer.appendChild(heading);

      var list = document.createElement("ul");
      list.className = "nl-answer__caveats";
      caveats.forEach(function (caveat) {
        var item = document.createElement("li");
        item.textContent = caveat;
        list.appendChild(item);
      });
      answer.appendChild(list);
    }
    answer.hidden = false;

    var query = result.query ? "?" + result.query : "";
    urlInput.value = "GET /" + result.target + query;
    /* The builder listens for `change` to re-derive its rows from the URL, so
     * both modes show the same query. Programmatic assignment does not fire
     * it. */
    urlInput.dispatchEvent(new Event("change", { bubbles: true }));
    urlInput.focus();
  }

  /* Pulls the human-readable text out of an OperationOutcome. */
  function outcomeText(body) {
    return (
      (body &&
        body.issue &&
        body.issue[0] &&
        (body.issue[0].diagnostics ||
          (body.issue[0].details && body.issue[0].details.text))) ||
      messages.msgError
    );
  }

  function translate() {
    var value = (text.value || "").trim();
    if (!value) {
      text.focus();
      return;
    }

    submit.disabled = true;
    showMessage("nl-answer--working", messages.msgWorking);

    fetch("/$nl-search", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Accept: "application/json",
      },
      body: JSON.stringify({ text: value }),
    })
      .then(function (response) {
        return response
          .json()
          .catch(function () {
            return null;
          })
          .then(function (body) {
            return { ok: response.ok, body: body };
          });
      })
      .then(function (result) {
        if (!result.ok) {
          showMessage("nl-answer--error", outcomeText(result.body));
          return;
        }
        if (!result.body || !result.body.supported) {
          showMessage(
            "nl-answer--unsupported",
            (result.body && result.body.reason) || messages.msgUnsupported
          );
          return;
        }
        showTranslation(result.body);
      })
      .catch(function () {
        showMessage("nl-answer--error", messages.msgError);
      })
      .then(function () {
        submit.disabled = false;
      });
  }

  submit.addEventListener("click", translate);
  text.addEventListener("keydown", function (event) {
    if (event.key === "Enter") {
      event.preventDefault();
      translate();
    }
  });

  pane.addEventListener("click", function (event) {
    var chip = event.target.closest("[data-nl-example]");
    if (!chip) return;
    text.value = chip.textContent.trim();
    translate();
  });
})();
