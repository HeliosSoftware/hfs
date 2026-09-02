/* Incremental CapabilityStatement JSON only. Shared JSON viewers are untouched. */
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

  function rootFor(details) {
    return details.closest("#capability-json-fold");
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

  function expandNext(root) {
    if (!root || root.dataset.capabilityJsonExpandAll !== "true") return;
    var nodes = root.querySelectorAll("details[data-capability-json-node]");
    for (var i = 0; i < nodes.length; i++) {
      var node = nodes[i];
      if (node.open) continue;
      node.open = true;
      var body = bodyFor(node);
      if (body) {
        triggerLoad(body);
        return;
      }
    }
  }

  function setAll(details, expand) {
    var root = rootFor(details);
    if (!root) return;
    root.dataset.capabilityJsonExpandAll = expand ? "true" : "false";
    if (expand) {
      root.open = true;
      var rootBody = bodyFor(root);
      if (rootBody) {
        triggerLoad(rootBody);
        expandNext(root);
      }
      return;
    }
    root.querySelectorAll("details[data-capability-json-node]").forEach(function (node) {
      collapse(node);
    });
  }

  document.addEventListener(
    "toggle",
    function (event) {
      var details = event.target;
      if (!details.matches || !details.matches("details[data-capability-json-node]")) return;
      var body = bodyFor(details);
      if (!body) return;

      if (details.open) {
        triggerLoad(body);
      } else {
        // Removing the swapped subtree is the DOM budget: reopening issues a
        // fresh, bounded request rather than retaining descendants invisibly.
        if (details === rootFor(details)) details.dataset.capabilityJsonExpandAll = "false";
        reset(body, false);
      }
    },
    true,
  );

  document.addEventListener("click", function (event) {
    var control = event.target.closest && event.target.closest("[data-capability-json-fold]");
    if (!control) return;
    event.preventDefault();
    event.stopPropagation();
    var details = control.closest("#capability-json-fold");
    if (!details) return;
    setAll(details, control.dataset.capabilityJsonFold === "none");
  });

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
    expandNext(rootFor(target));
  });

  document.addEventListener("htmx:afterRequest", function (event) {
    var target = event.detail && event.detail.target;
    if (!target || !target.hasAttribute("data-capability-json-body")) return;
    target.capabilityJsonXhr = null;
    var status = event.detail.xhr && event.detail.xhr.status;
    if (!(status >= 200 && status < 400)) {
      var details = target.closest("details[data-capability-json-node]");
      reset(target, !!(details && details.open));
      var root = rootFor(target);
      if (root) root.dataset.capabilityJsonExpandAll = "false";
    }
  });
})();
