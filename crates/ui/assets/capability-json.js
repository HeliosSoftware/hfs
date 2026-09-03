/* Bounded CapabilityStatement JSON tree, shared by HFS and HTS. */
(function () {
  "use strict";

  var MAX_ROWS = 1000;
  var MAX_BYTES = 1024 * 1024;
  var MAX_PAGES = 64;
  var ROOT_SELECTOR = "[data-capability-json-root]";

  function rootFor(element) {
    return element && element.closest ? element.closest(ROOT_SELECTOR) : null;
  }

  function regionFor(root) {
    return root && root.querySelector("#capability-json-body");
  }

  function treeFor(root) {
    return root && root.querySelector("[data-capability-json-tree]");
  }

  function bodyFor(details) {
    for (var i = 0; i < details.children.length; i += 1) {
      if (details.children[i].hasAttribute("data-capability-json-body")) {
        return details.children[i];
      }
    }
    return null;
  }

  function utf8Bytes(value) {
    if (window.TextEncoder) return new TextEncoder().encode(value).length;
    return unescape(encodeURIComponent(value)).length;
  }

  function withinBudget(tree) {
    return (
      tree.querySelectorAll(".capability-json-row").length <= MAX_ROWS &&
      utf8Bytes(tree.innerHTML) <= MAX_BYTES
    );
  }

  function hasRoom(tree) {
    return (
      tree.querySelectorAll(".capability-json-row").length < MAX_ROWS &&
      utf8Bytes(tree.innerHTML) < MAX_BYTES
    );
  }

  function nodePath(ancestor, node) {
    var path = [];
    while (node && node !== ancestor) {
      var parent = node.parentNode;
      if (!parent) return null;
      path.push(Array.prototype.indexOf.call(parent.childNodes, node));
      node = parent;
    }
    return node === ancestor ? path.reverse() : null;
  }

  function nodeAtPath(ancestor, path) {
    var node = ancestor;
    for (var i = 0; i < path.length; i += 1) {
      node = node.childNodes[path[i]];
      if (!node) return null;
    }
    return node;
  }

  function prospectiveFits(tree, target, response) {
    var path = nodePath(tree, target);
    if (!path) return false;
    var clone = tree.cloneNode(true);
    var cloneTarget = nodeAtPath(clone, path);
    if (!cloneTarget) return false;
    cloneTarget.innerHTML = response;
    return withinBudget(clone);
  }

  function responseFits(response) {
    if (utf8Bytes(response) > MAX_BYTES) return false;
    var template = document.createElement("template");
    template.innerHTML = response;
    return (
      template.content.querySelectorAll(".capability-json-row").length <= MAX_ROWS &&
      utf8Bytes(template.innerHTML) <= MAX_BYTES
    );
  }

  function setStatus(root, message) {
    var status = root.querySelector("[data-capability-json-status]");
    if (status) status.textContent = message || "";
  }

  function text(root, name) {
    return root.dataset[name] || "";
  }

  function statusNode(message, failed, spinning) {
    var node = document.createElement("p");
    node.className = failed ? "notice notice--warn" : "busy-status";
    node.setAttribute("role", "status");
    if (spinning) {
      var spinner = document.createElement("span");
      spinner.className = "spinner";
      spinner.setAttribute("aria-hidden", "true");
      node.appendChild(spinner);
    }
    node.appendChild(document.createTextNode(message || ""));
    return node;
  }

  function clearBodySnapshot(body) {
    delete body.capabilityJsonPreviousHtml;
    delete body.capabilityJsonPreviousLoaded;
  }

  function clearBodyLifecycle(body) {
    body.capabilityJsonXhr = null;
    body.dataset.capabilityJsonLoading = "false";
    body.removeAttribute("aria-busy");
    delete body.dataset.capabilityJsonFocusDirection;
  }

  function resetBody(body, failed) {
    body.replaceChildren(
      statusNode(
        failed ? body.dataset.errorText : body.dataset.loadingText,
        failed,
        !failed,
      ),
    );
    body.dataset.capabilityJsonLoaded = "false";
    clearBodyLifecycle(body);
    clearBodySnapshot(body);
  }

  function restoreBody(body, outcome, message) {
    if (
      body.capabilityJsonPreviousLoaded === true &&
      typeof body.capabilityJsonPreviousHtml === "string"
    ) {
      body.innerHTML = body.capabilityJsonPreviousHtml;
      body.dataset.capabilityJsonLoaded = "true";
    } else if (outcome === "error") {
      resetBody(body, true);
      return;
    } else if (outcome === "limit") {
      body.replaceChildren(statusNode(message, true, false));
      body.dataset.capabilityJsonLoaded = "false";
    } else {
      resetBody(body, false);
      return;
    }
    clearBodyLifecycle(body);
    clearBodySnapshot(body);
  }

  function completeBody(body) {
    body.dataset.capabilityJsonLoaded = "true";
    clearBodyLifecycle(body);
    clearBodySnapshot(body);
  }

  function updateControls(root) {
    var tree = treeFor(root);
    var expand = root.querySelector("[data-capability-json-expand-all]");
    var collapse = root.querySelector("[data-capability-json-collapse-all]");
    if (!tree || !expand || !collapse) return;
    var busy = !!root.capabilityJsonAggregateController;
    var state = tree.firstElementChild && tree.firstElementChild.dataset.expansionState;
    var hasExpandable = !!tree.querySelector(
      "details[data-capability-json-node] [data-capability-json-body]",
    );
    expand.disabled = busy || state === "complete" || !hasExpandable;
    collapse.disabled = busy ? false : tree.innerHTML === root.capabilityJsonInitialMarkup;
  }

  function queue(root, item) {
    root.capabilityJsonQueue = root.capabilityJsonQueue || [];
    var duplicate = root.capabilityJsonQueue.some(function (queued) {
      return queued.kind === item.kind && queued.element === item.element;
    });
    if (!duplicate) root.capabilityJsonQueue.push(item);
  }

  function drainQueue(root) {
    if (root.capabilityJsonAggregateController || root.capabilityJsonActiveBody) return;
    var queued = root.capabilityJsonQueue || [];
    while (queued.length) {
      var item = queued.shift();
      if (!item.element || !item.element.isConnected) continue;
      if (item.element.closest("details[data-capability-json-node]:not([open])")) {
        continue;
      }
      if (item.kind === "body") {
        var details = item.element.closest("details[data-capability-json-node]");
        if (details && details.open) requestBody(root, item.element);
      } else {
        item.element.click();
      }
      break;
    }
  }

  function finishManual(root, body) {
    if (!body.capabilityJsonRequestActive && root.capabilityJsonActiveBody !== body) {
      return;
    }
    body.capabilityJsonRequestActive = false;
    if (root.capabilityJsonActiveBody === body) root.capabilityJsonActiveBody = null;
    updateControls(root);
    drainQueue(root);
  }

  function cancelBody(root, body) {
    var xhr = body.capabilityJsonXhr;
    var inFlight =
      body.capabilityJsonRequestActive ||
      body.dataset.capabilityJsonLoading === "true" ||
      (xhr && xhr.readyState !== 4);
    if (!inFlight) return;
    body.capabilityJsonCancelled = true;
    restoreBody(body, "cancel");
    finishManual(root, body);
    if (xhr && xhr.readyState !== 4) xhr.abort();
  }

  function cancelBodies(root, element) {
    element.querySelectorAll("[data-capability-json-body]").forEach(function (body) {
      cancelBody(root, body);
    });
  }

  function requestBody(root, body) {
    if (
      body.dataset.capabilityJsonLoaded === "true" ||
      body.dataset.capabilityJsonLoading === "true"
    ) {
      return;
    }
    var tree = treeFor(root);
    if (!hasRoom(tree)) {
      body.replaceChildren(statusNode(text(root, "limitText"), true, false));
      setStatus(root, text(root, "limitText"));
      return;
    }
    if (root.capabilityJsonAggregateController || root.capabilityJsonActiveBody) {
      queue(root, { kind: "body", element: body });
      return;
    }
    root.capabilityJsonActiveBody = body;
    body.capabilityJsonRequestActive = true;
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
          // htmx lifecycle events retain or restore the usable tree.
        });
    } else {
      resetBody(body, false);
      finishManual(root, body);
    }
  }

  function cancelManual(root) {
    root.capabilityJsonQueue = [];
    var region = regionFor(root);
    if (region) cancelBodies(root, region);
    root.capabilityJsonActiveBody = null;
  }

  function collectPages(tree) {
    var pages = Array.prototype.slice.call(
      tree.querySelectorAll("[data-capability-json-page]"),
    );
    if (pages.length > MAX_PAGES) return null;
    var body = new URLSearchParams();
    pages.forEach(function (page) {
      body.append("path", page.dataset.path || "");
      body.append("offset", page.dataset.offset || "0");
      body.append("limit", page.dataset.limit || "100");
    });
    return body;
  }

  function stopAggregate(root) {
    root.capabilityJsonGeneration = (root.capabilityJsonGeneration || 0) + 1;
    if (root.capabilityJsonAggregateController) {
      root.capabilityJsonAggregateController.abort();
      root.capabilityJsonAggregateController = null;
    }
    var region = regionFor(root);
    if (region) region.removeAttribute("aria-busy");
  }

  function expandAll(root) {
    var tree = treeFor(root);
    var region = regionFor(root);
    if (!tree || !region || root.capabilityJsonAggregateController) return;
    var body = collectPages(tree);
    if (!body) {
      setStatus(root, text(root, "limitText"));
      return;
    }

    cancelManual(root);
    var controller = new AbortController();
    var generation = (root.capabilityJsonGeneration || 0) + 1;
    root.capabilityJsonGeneration = generation;
    root.capabilityJsonAggregateController = controller;
    region.setAttribute("aria-busy", "true");
    setStatus(root, text(root, "expandingText"));
    updateControls(root);

    fetch(root.dataset.expandUrl, {
      method: "POST",
      headers: {
        Accept: "text/html",
        "Content-Type": "application/x-www-form-urlencoded;charset=UTF-8",
      },
      body: body.toString(),
      signal: controller.signal,
    })
      .then(function (response) {
        if (!response.ok) throw new Error("HTTP " + response.status);
        return response.text();
      })
      .then(function (html) {
        if (
          controller.signal.aborted ||
          root.capabilityJsonGeneration !== generation ||
          root.capabilityJsonAggregateController !== controller
        ) {
          return;
        }
        if (!responseFits(html)) {
          setStatus(root, text(root, "limitText"));
          return;
        }
        tree.innerHTML = html;
        if (window.htmx && typeof window.htmx.process === "function") {
          window.htmx.process(tree);
        }
        var state = tree.firstElementChild && tree.firstElementChild.dataset.expansionState;
        setStatus(
          root,
          state === "complete" ? text(root, "completeText") : text(root, "partialText"),
        );
      })
      .catch(function (error) {
        if (error.name !== "AbortError" && root.capabilityJsonGeneration === generation) {
          setStatus(root, text(root, "errorText"));
        }
      })
      .finally(function () {
        if (root.capabilityJsonAggregateController === controller) {
          root.capabilityJsonAggregateController = null;
          region.removeAttribute("aria-busy");
          updateControls(root);
          drainQueue(root);
        }
      });
  }

  function collapseAll(root, control) {
    var tree = treeFor(root);
    if (!tree) return;
    stopAggregate(root);
    cancelManual(root);
    tree.innerHTML = root.capabilityJsonInitialMarkup;
    if (window.htmx && typeof window.htmx.process === "function") {
      window.htmx.process(tree);
    }
    setStatus(root, "");
    updateControls(root);
    if (control && control.isConnected) control.focus({ preventScroll: true });
  }

  function enhance(root) {
    if (root.dataset.capabilityJsonEnhanced === "true") return;
    var tree = treeFor(root);
    var actions = root.querySelector("[data-capability-json-actions]");
    var initial = tree && tree.querySelector('[data-capability-json-page][data-path=""]');
    if (!tree || !actions || !initial) return;
    root.dataset.capabilityJsonEnhanced = "true";
    root.capabilityJsonInitialMarkup = tree.innerHTML;
    root.capabilityJsonGeneration = 0;
    root.capabilityJsonQueue = [];
    actions.hidden = false;
    var fallback = root.querySelector("[data-capability-json-fallback]");
    if (fallback) fallback.hidden = true;
    updateControls(root);
  }

  function enhanceAll(scope) {
    if (scope.matches && scope.matches(ROOT_SELECTOR)) enhance(scope);
    scope.querySelectorAll(ROOT_SELECTOR).forEach(enhance);
  }

  document.addEventListener("click", function (event) {
    var expand = event.target.closest("[data-capability-json-expand-all]");
    if (expand) {
      expandAll(rootFor(expand));
      return;
    }
    var collapse = event.target.closest("[data-capability-json-collapse-all]");
    if (collapse) collapseAll(rootFor(collapse), collapse);
  });

  document.addEventListener(
    "toggle",
    function (event) {
      var details = event.target;
      if (!details.matches || !details.matches("details[data-capability-json-node]")) return;
      var root = rootFor(details);
      var body = bodyFor(details);
      if (!root || !body) return;
      if (details.open) {
        requestBody(root, body);
      } else {
        cancelBodies(root, body);
        cancelBody(root, body);
        resetBody(body, false);
        finishManual(root, body);
      }
      updateControls(root);
    },
    true,
  );

  document.addEventListener("htmx:beforeRequest", function (event) {
    var target = event.detail && event.detail.target;
    if (!target || !target.hasAttribute("data-capability-json-body")) return;
    var root = rootFor(target);
    var region = regionFor(root);
    if (!root || target === region) return;
    var source =
      (event.detail.requestConfig && event.detail.requestConfig.elt) ||
      event.detail.elt ||
      event.target;
    if (
      root.capabilityJsonAggregateController ||
      (root.capabilityJsonActiveBody && root.capabilityJsonActiveBody !== target)
    ) {
      event.preventDefault();
      queue(root, {
        kind: source && source.dataset.capabilityJsonPageDirection ? "source" : "body",
        element: source && source.dataset.capabilityJsonPageDirection ? source : target,
      });
      return;
    }
    root.capabilityJsonActiveBody = target;
    target.capabilityJsonRequestActive = true;
    delete target.capabilityJsonCancelled;
    delete target.capabilityJsonBudgetRejected;
    target.capabilityJsonPreviousHtml = target.innerHTML;
    target.capabilityJsonPreviousLoaded = target.dataset.capabilityJsonLoaded === "true";
    target.dataset.capabilityJsonLoading = "true";
    target.setAttribute("aria-busy", "true");
    target.capabilityJsonXhr = event.detail.xhr;
    if (source && source.dataset.capabilityJsonPageDirection) {
      target.dataset.capabilityJsonFocusDirection = source.dataset.capabilityJsonPageDirection;
    }
  });

  document.addEventListener("htmx:beforeSwap", function (event) {
    var target = event.detail && event.detail.target;
    if (!target || !target.hasAttribute("data-capability-json-body")) return;
    var root = rootFor(target);
    var tree = treeFor(root);
    var response =
      (event.detail && event.detail.serverResponse) ||
      (event.detail.xhr && event.detail.xhr.responseText) ||
      "";
    if (!root || !tree || prospectiveFits(tree, target, response)) return;
    event.detail.shouldSwap = false;
    target.capabilityJsonBudgetRejected = true;
    restoreBody(target, "limit", text(root, "limitText"));
    setStatus(root, text(root, "limitText"));
  });

  document.addEventListener("htmx:afterSwap", function (event) {
    var target = event.detail && event.detail.target;
    if (!target || !target.hasAttribute("data-capability-json-body")) return;
    var root = rootFor(target);
    var tree = treeFor(root);
    var details = target.closest("details[data-capability-json-node]");
    if (!root || !tree) return;
    if (!withinBudget(tree)) {
      target.capabilityJsonBudgetRejected = true;
      restoreBody(target, "limit", text(root, "limitText"));
      setStatus(root, text(root, "limitText"));
    } else if (details && !details.open) {
      resetBody(target, false);
    } else {
      var direction = target.dataset.capabilityJsonFocusDirection;
      completeBody(target);
      if (direction) {
        var preferred = target.querySelector(
          '[data-capability-json-page-direction="' + direction + '"]:not(:disabled)',
        );
        var fallback = target.querySelector(
          "[data-capability-json-page-direction]:not(:disabled)",
        );
        if (preferred || fallback) (preferred || fallback).focus({ preventScroll: true });
      }
    }
    updateControls(root);
  });

  document.addEventListener("htmx:afterRequest", function (event) {
    var target = event.detail && event.detail.target;
    if (!target || !target.hasAttribute("data-capability-json-body")) return;
    var root = rootFor(target);
    if (!root) return;
    var status = event.detail.xhr && event.detail.xhr.status;
    if (target.capabilityJsonCancelled) {
      delete target.capabilityJsonCancelled;
    } else if (target.capabilityJsonBudgetRejected) {
      delete target.capabilityJsonBudgetRejected;
    } else if (!(status >= 200 && status < 400)) {
      restoreBody(target, "error");
      setStatus(root, text(root, "errorText"));
    } else if (target.dataset.capabilityJsonLoading === "true") {
      restoreBody(target, "error");
      setStatus(root, text(root, "errorText"));
    }
    finishManual(root, target);
  });

  document.addEventListener("htmx:load", function (event) {
    enhanceAll(event.detail && event.detail.elt ? event.detail.elt : document);
  });

  enhanceAll(document);
})();
