/* Incremental JSON fold rendering — shared by CapabilityStatement and raw response folds.
   Each fold is a <details data-json-fold-root> with nested <details data-capability-json-node>
   containers. Multiple folds on one page operate independently (#898). */
(function () {
  "use strict";

  function bodyFor(details) {
    for (var i = 0; i < details.children.length; i++) {
      if (details.children[i].hasAttribute("data-capability-json-body")) {
        return details.children[i];
      }
    }
    return null;
  }

  function status(body, failed) {
    var message = document.createElement("p");
    message.className = failed ? "notice notice--warn" : "busy-status";
    message.setAttribute("role", "status");
    if (!failed) {
      var spinner = document.createElement("span");
      spinner.className = "spinner";
      spinner.setAttribute("aria-hidden", "true");
      message.appendChild(spinner);
    }
    message.appendChild(
      document.createTextNode(
        failed ? body.dataset.errorText || "" : body.dataset.loadingText || "",
      ),
    );
    return message;
  }

  function abortBody(body) {
    var xhr = body.capabilityJsonXhr;
    body.capabilityJsonXhr = null;
    if (xhr && xhr.readyState !== 4) xhr.abort();
  }

  function abortTree(body) {
    abortBody(body);
    body.querySelectorAll("[data-capability-json-body]").forEach(function (descendant) {
      abortBody(descendant);
    });
  }

  function reset(body, failed) {
    // Abort from the outside in while every descendant body is still reachable.
    // Replacing the children afterward cannot leave a detached request able to
    // swap stale content back into an ancestor that was already collapsed.
    abortTree(body);
    body.replaceChildren(status(body, failed));
    body.dataset.capabilityJsonLoaded = "false";
    body.dataset.capabilityJsonLoading = "false";
    delete body.dataset.capabilityJsonFocusDirection;
    body.removeAttribute("aria-busy");
  }

  function collapse(details) {
    if (!details || !details.isConnected) return;
    details.open = false;
    var body = bodyFor(details);
    if (body) reset(body, false);
  }

  function closeOtherBranches(details) {
    // Find the nearest fold root (supports multiple folds per page, #898)
    var root = details.closest("[data-json-fold-root]");
    if (!root || details === root) return;
    root.querySelectorAll("details[data-capability-json-node][open]").forEach(function (open) {
      if (open === details || open === root || open.contains(details)) return;
      collapse(open);
    });
  }

  function triggerLoad(body) {
    if (
      body.dataset.capabilityJsonLoaded === "true" ||
      body.dataset.capabilityJsonLoading === "true"
    ) {
      return;
    }
    body.dataset.capabilityJsonLoading = "true";
    body.setAttribute("aria-busy", "true");
    if (window.htmx && typeof window.htmx.ajax === "function") {
      window.htmx
        .ajax("GET", body.dataset.fragmentUrl, {
          source: body,
          target: body,
          swap: "innerHTML",
        })
        .catch(function () {
          // Lifecycle events below restore a localized, retryable state.
        });
    }
  }

  document.addEventListener(
    "toggle",
    function (event) {
      var details = event.target;
      if (!details.matches || !details.matches("details[data-capability-json-node]")) return;
      var body = bodyFor(details);
      if (!body) return;

      if (details.open) {
        closeOtherBranches(details);
        triggerLoad(body);
      } else {
        // Removing the swapped subtree is the DOM budget: reopening issues a
        // fresh, bounded request rather than retaining descendants invisibly.
        reset(body, false);
      }
    },
    true,
  );

  document.addEventListener("htmx:beforeRequest", function (event) {
    var target = event.detail && event.detail.target;
    if (!target || !target.hasAttribute("data-capability-json-body")) return;
    target.dataset.capabilityJsonLoading = "true";
    target.setAttribute("aria-busy", "true");
    target.capabilityJsonXhr = event.detail.xhr;
    var source =
      (event.detail.requestConfig && event.detail.requestConfig.elt) ||
      event.detail.elt ||
      event.target;
    if (source && source.dataset.capabilityJsonPageDirection) {
      target.dataset.capabilityJsonFocusDirection =
        source.dataset.capabilityJsonPageDirection;
    }
  });

  document.addEventListener("htmx:afterSwap", function (event) {
    var target = event.detail && event.detail.target;
    if (!target || !target.hasAttribute("data-capability-json-body")) return;
    var details = target.closest("details[data-capability-json-node]");
    if (details && !details.open) {
      reset(target, false);
      return;
    }
    target.dataset.capabilityJsonLoaded = "true";
    target.dataset.capabilityJsonLoading = "false";
    target.removeAttribute("aria-busy");
    target.capabilityJsonXhr = null;
    var direction = target.dataset.capabilityJsonFocusDirection;
    delete target.dataset.capabilityJsonFocusDirection;
    if (direction) {
      var preferred = target.querySelector(
        '[data-capability-json-page-direction="' + direction + '"]:not(:disabled)',
      );
      var fallback = target.querySelector(
        "[data-capability-json-page-direction]:not(:disabled)",
      );
      var control = preferred || fallback;
      if (control) control.focus({ preventScroll: true });
    }
  });

  document.addEventListener("htmx:afterRequest", function (event) {
    var target = event.detail && event.detail.target;
    if (!target || !target.hasAttribute("data-capability-json-body")) return;
    target.capabilityJsonXhr = null;
    var status = event.detail.xhr && event.detail.xhr.status;
    if (!(status >= 200 && status < 400)) {
      var details = target.closest("details[data-capability-json-node]");
      reset(target, !!(details && details.open));
    }
  });
})();
