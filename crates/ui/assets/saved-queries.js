/*
 * Saved FHIR queries & search builder (issue #234), backed by the per-user
 * settings document and the FHIR REST API itself.
 *
 * The page shell is server-rendered; this script owns three client concerns:
 *
 * 1. The read/modify/write cycle against /_user/settings. Unlike theme.js (a
 *    single last-write-wins scalar), saved queries are structural state
 *    shared across tabs and devices, so every write is a JSON merge patch
 *    conditional on the document's ETag, retried once against a fresh read
 *    when another writer won the race (412).
 * 2. The visual builder: condition / include / result-control rows kept in
 *    two-way sync with the GET URL. Parameter suggestions come from
 *    /ui/queries/params — a server-rendered datalist fragment fed by the
 *    SearchParameter registry, swapped per resource type.
 * 3. Results: Run fetches the search from the FHIR API (the existing REST
 *    surface, no UI-facing endpoint) and renders Bundle.total, a table whose
 *    columns honor _elements, and paging over Bundle.link.
 *
 * Document conventions (see helios-persistence's user_settings module docs):
 * - savedQueries.<ResourceType>.<id> = { name, query, createdAt,
 *   lastAccessedAt?, accessCount? }. Keyed by id precisely so a merge patch
 *   can touch one entry without clobbering siblings.
 * - recentSearches = [{ query: "/Patient?name=smith", at: ISO }] — newest
 *   first, deduped by query, capped. An array (replaced wholesale by every
 *   merge patch) is fine here: it is a small bounded cache rewritten on each
 *   run, not sibling-keyed state.
 */
(function () {
  "use strict";

  var SETTINGS = "/_user/settings";
  /* The effective tenant, stamped by the server (#344); FHIR calls carry it. */
  var TENANT = (document.querySelector('meta[name="hfs-tenant"]') || {}).content || "";
  function fhirHeaders() {
    var h = { Accept: "application/fhir+json" };
    if (TENANT) h["X-Tenant-ID"] = TENANT;
    return h;
  }
  var MAX_RETRIES = 2;
  var MAX_RECENT = 10;

  /* The builder, the results table and the recent list are shared with the
   * Search page (#255), which renders the same partials; the saved-query list
   * is this page's alone. So the script keys off the form, and treats the list
   * host as optional. Prose comes from the shared message carrier either page
   * renders. */
  var root = document.getElementById("saved-queries");
  var messageHost = document.getElementById("search-messages");
  var errorHost = document.getElementById("search-error");
  var form = document.getElementById("saved-query-form");
  var recentHost = document.getElementById("recent-searches");
  var sections = document.getElementById("builder-sections");
  var urlInput = form && form.elements.url;
  if (!form || !messageHost || !window.fetch) return;

  var messages = messageHost.dataset;
  var etag = null;
  var lang = document.documentElement.lang || undefined;

  function fetchDocument() {
    return fetch(SETTINGS, {
      headers: { Accept: "application/json" },
      credentials: "same-origin",
    }).then(function (response) {
      if (response.status === 501) throw { unavailable: true };
      if (!response.ok) throw new Error("settings fetch failed");
      etag = response.headers.get("ETag");
      return response.json();
    });
  }

  /* Merge-patches the document, conditional on the last ETag we saw; on 412
   * re-reads and retries so an unrelated concurrent write (another tab, the
   * theme toggle) never surfaces to the user. */
  function patchDocument(patch, attempt) {
    var headers = { "Content-Type": "application/json" };
    if (etag) headers["If-Match"] = etag;

    return fetch(SETTINGS, {
      method: "PATCH",
      headers: headers,
      credentials: "same-origin",
      body: JSON.stringify(patch),
    }).then(function (response) {
      if (response.status === 412 && attempt < MAX_RETRIES) {
        return fetchDocument().then(function () {
          return patchDocument(patch, attempt + 1);
        });
      }
      if (!response.ok) {
        return response
          .json()
          .catch(function () {
            return null;
          })
          .then(function (outcome) {
            throw { outcome: outcome };
          });
      }
      etag = response.headers.get("ETag");
      return response.json();
    });
  }

  /* Scopes a merge patch to one saved-query entry (or null to delete it). */
  function patchEntry(resourceType, id, entry) {
    var patch = { savedQueries: {} };
    patch.savedQueries[resourceType] = {};
    patch.savedQueries[resourceType][id] = entry;
    return patchDocument(patch, 0);
  }

  function savedQueries(doc) {
    var byType = doc && doc.savedQueries;
    return byType && typeof byType === "object" && !Array.isArray(byType)
      ? byType
      : {};
  }

  function recentSearches(doc) {
    var list = doc && doc.recentSearches;
    if (!Array.isArray(list)) return [];
    return list
      .filter(function (item) {
        return item && typeof item.query === "string";
      })
      .slice(0, MAX_RECENT); // the cap is enforced on write; re-assert on read
  }

  /* Accepts "GET /Patient?name=smith", "/Patient?...", or an absolute URL;
   * the resource type comes from the path. Returns null when it cannot. */
  function parseSearchUrl(raw) {
    var text = (raw || "").trim().replace(/^GET\s+/i, "");
    if (/^https?:\/\//i.test(text)) {
      try {
        var url = new URL(text);
        text = url.pathname + url.search;
      } catch (e) {
        return null;
      }
    }
    var match = /^\/?([A-Za-z]+)(?:\?(.*))?$/.exec(text);
    if (!match) return null;
    return { type: match[1], query: (match[2] || "").trim() };
  }

  function searchPath(resourceType, query) {
    return (
      "/" + encodeURIComponent(resourceType) + (query ? "?" + query : "")
    );
  }

  /* ---- Visual builder: rows kept in two-way sync with the GET URL ------ */

  var CONTROL_KEYS = ["_count", "_sort", "_total", "_summary", "_elements"];
  var INCLUDE_KEYS = ["_include", "_revinclude"];
  var COLON_MODIFIERS = [
    "exact", "contains", "missing", "not", "text",
    "above", "below", "in", "not-in", "identifier", "of-type",
  ];
  /* Comparator prefixes live on the value (ge2020-01-01), not the key. */
  var PREFIXES = ["eq", "ne", "gt", "ge", "lt", "le", "sa", "eb", "ap"];
  var PREFIX_RE = /^(eq|ne|gt|ge|lt|le|sa|eb|ap)(?=[\d])/;
  var catalogType = null;

  /* Per-type parameter metadata from the catalog fragment's data attributes:
   * type -> { code: { type: "reference"|..., targets: ["Patient", ...] } }.
   * Feeds the chaining controls; promise-cached so each type is fetched once. */
  var PARAM_META = {};
  var paramMetaPromises = {};
  function parseCatalog(html) {
    var tpl = document.createElement("template");
    tpl.innerHTML = html;
    var meta = {};
    tpl.content.querySelectorAll("option").forEach(function (opt) {
      meta[opt.value] = {
        type: opt.dataset.type || "",
        targets: (opt.dataset.targets || "").split(",").filter(Boolean),
      };
    });
    return { meta: meta, datalist: tpl.content.querySelector("datalist") };
  }
  function fetchParams(type) {
    if (!type) return Promise.resolve({});
    if (paramMetaPromises[type]) return paramMetaPromises[type];
    paramMetaPromises[type] = fetch(
      "/ui/queries/params?type=" + encodeURIComponent(type),
      { credentials: "same-origin" },
    )
      .then(function (response) {
        return response.ok ? response.text() : null;
      })
      .then(function (html) {
        if (!html) return {};
        var parsed = parseCatalog(html);
        PARAM_META[type] = parsed.meta;
        return parsed.meta;
      });
    return paramMetaPromises[type];
  }

  /* Swaps the parameter datalist for the current resource type. The
   * fragment is server-rendered from the SearchParameter registry. */
  function loadCatalog(type) {
    if (!type || type === catalogType) return;
    catalogType = type;
    fetch("/ui/queries/params?type=" + encodeURIComponent(type), {
      credentials: "same-origin",
    })
      .then(function (response) {
        return response.ok ? response.text() : null;
      })
      .then(function (html) {
        if (!html) return;
        var parsed = parseCatalog(html);
        PARAM_META[type] = parsed.meta;
        var current = document.getElementById("param-options");
        if (parsed.datalist && current) current.replaceWith(parsed.datalist);
        refreshChainAffordances();
      });
  }

  function splitQuery(query) {
    return (query || "")
      .split("&")
      .filter(Boolean)
      .map(function (pair) {
        var eq = pair.indexOf("=");
        var rawKey = eq < 0 ? pair : pair.slice(0, eq);
        var value = eq < 0 ? "" : pair.slice(eq + 1);
        try {
          value = decodeURIComponent(value.replace(/\+/g, " "));
        } catch (e) {
          /* keep the raw value */
        }
        return parseKey(rawKey, value);
      });
  }

  /* Parses one query key into a row part. Chained keys get their own kinds;
   * anything deeper than one hop (or a nested _has) stays a raw condition so
   * the URL keeps working even where the builder has no dedicated controls. */
  function parseKey(rawKey, value) {
    if (rawKey.indexOf("_has:") === 0) {
      /* _has:Type:ref-param:leaf[:modifier] — reverse chain, one level. */
      var segs = rawKey.split(":");
      var nested = segs.length > 3 && segs[3] === "_has";
      if (segs.length >= 4 && segs.length <= 5 && !nested && segs[3].indexOf(".") < 0) {
        return {
          kind: "has",
          hasType: segs[1],
          refParam: segs[2],
          key: segs[3],
          modifier: segs[4] || "",
          value: value,
        };
      }
      return { kind: "condition", key: rawKey, modifier: "", value: value };
    }
    var dot = rawKey.indexOf(".");
    if (dot > 0) {
      /* ref[:Type].ref[:Type]…​.leaf[:modifier] — forward chain, any depth.
       * Every segment but the last is a reference hop with an optional type
       * qualifier; the modifier belongs to the leaf. */
      var segs = rawKey.split(".");
      var hops = segs.slice(0, -1).map(function (seg) {
        var c = seg.indexOf(":");
        return {
          ref: c < 0 ? seg : seg.slice(0, c),
          type: c < 0 ? "" : seg.slice(c + 1),
        };
      });
      var last = segs[segs.length - 1];
      var lastColon = last.indexOf(":");
      return {
        kind: "chain",
        hops: hops,
        key: lastColon < 0 ? last : last.slice(0, lastColon),
        modifier: lastColon < 0 ? "" : last.slice(lastColon + 1),
        value: value,
      };
    }
    var colon = rawKey.indexOf(":");
    return {
      key: colon < 0 ? rawKey : rawKey.slice(0, colon),
      modifier: colon < 0 ? "" : rawKey.slice(colon + 1),
      value: value,
    };
  }

  function bucketFor(part) {
    if (part.kind === "chain" || part.kind === "has") return "condition";
    if (CONTROL_KEYS.indexOf(part.key) >= 0) return "control";
    if (INCLUDE_KEYS.indexOf(part.key) >= 0) return "include";
    return "condition";
  }

  function option(select, value, label, selected) {
    var el = document.createElement("option");
    el.value = value;
    el.textContent = label;
    el.selected = selected;
    select.appendChild(el);
  }

  /* ---- chaining rows (#394) -------------------------------------------- */

  var chainListSeq = 0;

  function pill(text) {
    var el = document.createElement("span");
    el.className = "builder-pill";
    el.textContent = text;
    return el;
  }
  function chainLabel(text) {
    var el = document.createElement("span");
    el.className = "builder-row__chainlabel";
    el.textContent = text;
    return el;
  }
  /* A free-text input backed by a per-row datalist we can refill. */
  function listedInput(row, placeholder, initial) {
    var input = document.createElement("input");
    var list = document.createElement("datalist");
    list.id = "chain-list-" + ++chainListSeq;
    input.setAttribute("list", list.id);
    input.placeholder = placeholder;
    input.spellcheck = false;
    input.value = initial || "";
    row.appendChild(list);
    return { input: input, list: list };
  }
  function fillList(list, codes) {
    list.textContent = "";
    codes.forEach(function (code) {
      var opt = document.createElement("option");
      opt.value = code;
      list.appendChild(opt);
    });
  }

  /* Modifier select + value input + remove button, shared by every row kind. */
  /* One value input inside a row's OR stack (#414). The per-value remove
   * only renders when the stack has siblings. */
  function orValueInput(host, value) {
    var wrap = document.createElement("span");
    wrap.className = "builder-row__orvalue";
    var input = document.createElement("input");
    input.className = "builder-row__value";
    input.value = value;
    input.placeholder = sections.dataset.msgValue;
    input.spellcheck = false;
    wrap.appendChild(input);
    var rm = document.createElement("button");
    rm.type = "button";
    rm.className = "builder-row__remove builder-row__remove--or";
    rm.dataset.removeOr = "true";
    rm.setAttribute("aria-label", sections.dataset.msgRemove);
    rm.textContent = "×";
    wrap.appendChild(rm);
    host.appendChild(wrap);
    return input;
  }

  /* Applicable colon modifiers per parameter type (#415); comparator
   * prefixes only apply to the ordered families. Unknown or unregistered
   * params keep the full lists. */
  var MODS_BY_TYPE = {
    string: ["exact", "contains", "missing"],
    token: ["text", "not", "in", "not-in", "of-type", "missing"],
    reference: ["identifier", "missing"],
    uri: ["below", "above", "missing"],
    date: ["missing"],
    number: ["missing"],
    quantity: ["missing"],
    composite: ["missing"],
    special: ["missing"],
  };
  var PREFIX_TYPES = ["date", "number", "quantity"];

  function applicableMods(paramType) {
    return MODS_BY_TYPE[paramType] || COLON_MODIFIERS;
  }
  function applicablePrefixes(paramType) {
    if (!paramType) return PREFIXES;
    return PREFIX_TYPES.indexOf(paramType) >= 0 ? PREFIXES : [];
  }

  /* Rebuilds a row's modifier select and MODIFY chips for its param type. */
  function refreshModifierControls(row, paramType) {
    var modifier = row.querySelector(".builder-row__modifier");
    if (!modifier) return;
    var selected = modifier.value;
    var mods = applicableMods(paramType);
    var prefixes = applicablePrefixes(paramType);
    modifier.textContent = "";
    option(modifier, "", sections.dataset.msgMatchIs, !selected);
    mods.forEach(function (m) {
      option(modifier, m, ":" + m, selected === m);
    });
    prefixes.forEach(function (p) {
      option(modifier, p, p, selected === p);
    });
    /* A modifier from a pasted URL stays selectable even when the type
     * would not offer it. */
    if (selected && mods.indexOf(selected) < 0 && prefixes.indexOf(selected) < 0) {
      option(modifier, selected, ":" + selected, true);
    }
    var panel = row.querySelector(".builder-row__modpanel");
    if (panel) fillModPanel(panel, row, mods);
  }

  /* Best-effort param type for a row's modifier target, from the cached
   * registry metadata. Empty string (unknown) keeps the full lists. */
  function rowParamType(row) {
    var base = sections.dataset.type || "";
    var leafEl = row.querySelector(".builder-row__cparam");
    if (row.classList.contains("builder-row--has")) {
      var t = row.querySelector(".builder-row__htype").value.trim();
      var leaf = leafEl.value.trim();
      return (((PARAM_META[t] || {})[leaf] || {}).type) || "";
    }
    if (row.classList.contains("builder-row--chain")) {
      var chainLeaf = leafEl.value.trim();
      for (var cached in PARAM_META) {
        var m = PARAM_META[cached][chainLeaf];
        if (m) return m.type;
      }
      return "";
    }
    var key = row.querySelector(".builder-row__key");
    if (!key) return "";
    return (((PARAM_META[base] || {})[key.value.trim()] || {}).type) || "";
  }

  /* Re-gates a row's modifier controls when its param type changed. */
  function regateModifiers(row) {
    var t = rowParamType(row);
    if (row.dataset.modType === t) return;
    row.dataset.modType = t;
    refreshModifierControls(row, t);
  }

  /* The MODIFY panel: each applicable modifier as a chip with its
   * plain-language explanation; clicking one selects it in the row. */
  function fillModPanel(panel, row, mods) {
    panel.textContent = "";
    var heading = document.createElement("span");
    heading.className = "builder-row__modheading";
    heading.textContent = sections.dataset.msgModifyHeading;
    panel.appendChild(heading);
    var current = row.querySelector(".builder-row__modifier").value;
    mods.forEach(function (m) {
      var chip = document.createElement("button");
      chip.type = "button";
      chip.className = "builder-row__modchip";
      chip.dataset.modChip = m;
      var code = document.createElement("strong");
      code.textContent = ":" + m;
      chip.appendChild(code);
      var desc = document.createElement("span");
      var msgKey = "msgMod" + m.replace(/(^|-)([a-z])/g, function (_, __, c) {
        return c.toUpperCase();
      });
      desc.textContent = sections.dataset[msgKey] || "";
      chip.appendChild(desc);
      chip.setAttribute("aria-pressed", current === m ? "true" : "false");
      panel.appendChild(chip);
    });
  }

  /* FHIR's OR is a comma list on one parameter (`name=Smith,Jones`): the
   * value column stacks one input per alternative, plus the `+ or` button. */
  function appendValues(row, rawValue) {
    var host = document.createElement("span");
    host.className = "builder-row__values";
    rawValue.split(",").forEach(function (v) {
      orValueInput(host, v);
    });
    row.appendChild(host);

    var addOr = document.createElement("button");
    addOr.type = "button";
    addOr.className = "builder-row__or";
    addOr.dataset.addOr = "true";
    addOr.textContent = sections.dataset.msgOr;
    row.appendChild(addOr);
  }

  function appendTail(row, part) {
    var value = part.value;
    var selectedMod = part.modifier;
    var prefix = PREFIX_RE.exec(value);
    if (!selectedMod && prefix) {
      selectedMod = prefix[1];
      value = value.slice(2);
    }
    var modifier = document.createElement("select");
    modifier.className = "builder-row__modifier";
    option(modifier, "", sections.dataset.msgMatchIs, !selectedMod);
    COLON_MODIFIERS.forEach(function (m) {
      option(modifier, m, ":" + m, selectedMod === m);
    });
    PREFIXES.forEach(function (p) {
      option(modifier, p, p, selectedMod === p);
    });
    row.appendChild(modifier);

    appendValues(row, value);

    var adv = document.createElement("button");
    adv.type = "button";
    adv.className = "builder-row__adv";
    adv.dataset.toggleMods = "true";
    adv.title = sections.dataset.msgModifyHeading;
    adv.setAttribute("aria-expanded", "false");
    adv.textContent = "⚙";
    row.appendChild(adv);

    var remove = document.createElement("button");
    remove.type = "button";
    remove.className = "builder-row__remove";
    remove.dataset.removeRow = "true";
    remove.setAttribute("aria-label", sections.dataset.msgRemove);
    remove.textContent = "×";
    row.appendChild(remove);

    var panel = document.createElement("div");
    panel.className = "builder-row__modpanel";
    panel.hidden = true;
    row.appendChild(panel);
  }

  /* Forward chain: one hop segment per reference —
   * `ref › [type]` … then the leaf param, operator, and value. Serializes to
   * `ref.leaf=`, `ref:Type.leaf=`, or deeper (`ref.ref2.leaf=`), matching the
   * numbered-levels panel in the design; the drill-deeper affordance appends
   * a hop when the leaf itself is a reference. */
  function chainRow(part) {
    var row = document.createElement("div");
    row.className = "builder-row builder-row--chain";
    var hops = (part.hops || []).map(function (h) {
      return { ref: h.ref, type: h.type };
    });

    var hopsHost = document.createElement("span");
    hopsHost.className = "builder-row__hops";
    row.appendChild(hopsHost);

    var leaf = listedInput(row, sections.dataset.msgParam, part.key);
    leaf.input.className = "builder-row__cparam";
    row.appendChild(leaf.input);

    var deeper = document.createElement("button");
    deeper.type = "button";
    deeper.className = "builder-row__drill";
    deeper.dataset.chainDeeper = "true";
    deeper.title = sections.dataset.msgChainInto;
    deeper.textContent = "›";
    deeper.hidden = true;
    row.appendChild(deeper);

    appendTail(row, part);

    var base = sections.dataset.type || "";

    /* Candidate resource types feeding hop k: the selected type of the
     * previous hop, else all of its registry targets (base type at k=0). */
    function parentTypes(k, done) {
      if (k === 0) return done([base]);
      var prev = hops[k - 1];
      if (prev.type) return done([prev.type]);
      parentTypes(k - 1, function (grand) {
        var pending = grand.length;
        var out = [];
        if (!pending) return done([]);
        grand.forEach(function (g) {
          fetchParams(g).then(function (meta) {
            (((meta[prev.ref] || {}).targets) || []).forEach(function (t) {
              if (out.indexOf(t) < 0) out.push(t);
            });
            if (--pending === 0) done(out);
          });
        });
      });
    }

    function segRefill(k) {
      var seg = hopsHost.children[k];
      if (!seg) return;
      var refInput = seg.querySelector(".builder-row__chainref");
      var typeSel = seg.querySelector(".builder-row__ctype");
      parentTypes(k, function (parents) {
        /* Ref-param suggestions: reference params across the parents. */
        var list = seg.querySelector("datalist");
        var pending = parents.length;
        var codes = [];
        if (!pending) return;
        parents.forEach(function (p) {
          fetchParams(p).then(function (meta) {
            Object.keys(meta).forEach(function (code) {
              if (meta[code].type === "reference" && codes.indexOf(code) < 0) {
                codes.push(code);
              }
            });
            if (--pending === 0) fillList(list, codes.sort());
          });
        });
        /* Target-type options for this hop's qualifier. */
        var chosen = hops[k].type;
        var tPending = parents.length;
        var targets = [];
        parents.forEach(function (p) {
          fetchParams(p).then(function (meta) {
            (((meta[refInput.value.trim()] || {}).targets) || []).forEach(function (t) {
              if (targets.indexOf(t) < 0) targets.push(t);
            });
            if (--tPending === 0) {
              typeSel.textContent = "";
              option(typeSel, "", sections.dataset.msgAnyTarget, chosen === "");
              targets.forEach(function (t) {
                option(typeSel, t, t, chosen === t);
              });
              if (chosen && targets.indexOf(chosen) < 0) {
                option(typeSel, chosen, chosen, true);
              }
            }
          });
        });
      });
    }

    function leafTypes(done) {
      parentTypes(hops.length, done);
    }

    function refillLeaf() {
      leafTypes(function (types) {
        var pending = types.length;
        var codes = [];
        var anyRef = false;
        if (!pending) return;
        types.forEach(function (t) {
          fetchParams(t).then(function (meta) {
            Object.keys(meta).forEach(function (code) {
              if (codes.indexOf(code) < 0) codes.push(code);
            });
            var m = meta[leaf.input.value.trim()];
            if (m && m.type === "reference") anyRef = true;
            if (--pending === 0) {
              fillList(leaf.list, codes.sort());
              deeper.hidden = !anyRef;
            }
          });
        });
      });
    }

    function addSegment(k) {
      var seg = document.createElement("span");
      seg.className = "builder-row__hopseg";

      var ref = listedInput(seg, sections.dataset.msgParam, hops[k].ref);
      ref.input.className = "builder-row__key builder-row__chainref";
      if (k === 0) ref.input.setAttribute("list", "param-options");
      seg.appendChild(ref.input);

      seg.appendChild(chainLabel("›"));

      var typeSel = document.createElement("select");
      typeSel.className = "builder-row__ctype";
      seg.appendChild(typeSel);

      ref.input.addEventListener("input", function () {
        hops[k].ref = ref.input.value.trim();
        segRefill(k);
        refillLeaf();
      });
      typeSel.addEventListener("change", function () {
        hops[k].type = typeSel.value;
        for (var j = k + 1; j < hops.length; j++) segRefill(j);
        refillLeaf();
        updateUrl();
      });

      hopsHost.appendChild(seg);
      segRefill(k);
    }

    hops.forEach(function (_, k) {
      addSegment(k);
    });
    refillLeaf();

    leaf.input.addEventListener("input", refillLeaf);
    deeper.addEventListener("click", function () {
      var refName = leaf.input.value.trim();
      if (!refName) return;
      hops.push({ ref: refName, type: "" });
      leaf.input.value = "";
      addSegment(hops.length - 1);
      refillLeaf();
      leaf.input.focus();
      updateUrl();
    });

    /* updateUrl reads the hops through the row's DOM state. */
    row.chainHops = hops;

    return row;
  }

  /* Reverse chain: has-a-related [Type] via [ref-param] where-its [param].
   * Serializes to `_has:Type:ref:param=` (one level). */
  function hasRow(part) {
    var row = document.createElement("div");
    row.className = "builder-row builder-row--has";

    row.appendChild(pill(sections.dataset.msgHasPill));

    var type = listedInput(row, sections.dataset.msgHasType, part.hasType);
    type.input.className = "builder-row__htype";
    type.input.setAttribute("list", "resource-type-options");
    row.appendChild(type.input);

    row.appendChild(chainLabel(sections.dataset.msgHasVia));

    var ref = listedInput(row, sections.dataset.msgParam, part.refParam);
    ref.input.className = "builder-row__href";
    row.appendChild(ref.input);

    row.appendChild(chainLabel(sections.dataset.msgHasWhere));

    var leaf = listedInput(row, sections.dataset.msgParam, part.key);
    leaf.input.className = "builder-row__cparam";
    row.appendChild(leaf.input);

    appendTail(row, part);

    var base = sections.dataset.type || "";
    function refillParams() {
      var t = type.input.value.trim();
      if (!t) return;
      fetchParams(t).then(function (meta) {
        var codes = Object.keys(meta).sort();
        /* The link must be a reference param that can point at the base
         * type; params with no declared targets stay offered. */
        fillList(
          ref.list,
          codes.filter(function (code) {
            var m = meta[code];
            if (m.type !== "reference") return false;
            return m.targets.length === 0 || m.targets.indexOf(base) >= 0;
          }),
        );
        fillList(leaf.list, codes);
      });
    }
    type.input.addEventListener("input", refillParams);
    refillParams();

    return row;
  }

  /* Related-data rows (#396): structured `_include` / `_revinclude` —
   * `Source : ref-param [: target]` plus an Iterate toggle, serializing to
   * `_include=Src:param[:Target]` (`:iterate` rides the key). A wildcard or
   * unparseable value falls back to the plain include row. */
  function includeRow(part) {
    var reverse = part.key === "_revinclude";
    var bits = (part.value || "").split(":");
    var row = document.createElement("div");
    row.className = "builder-row builder-row--include";
    row.dataset.includeKey = part.key;

    row.appendChild(pill(reverse ? "_revinclude" : "_includes"));

    var base = sections.dataset.type || "";
    var type = listedInput(row, sections.dataset.msgHasType, bits[0] || (reverse ? "" : base));
    type.input.className = "builder-row__itype";
    type.input.setAttribute("list", "resource-type-options");
    row.appendChild(type.input);

    row.appendChild(chainLabel(":"));

    var ref = listedInput(row, sections.dataset.msgParam, bits[1] || "");
    ref.input.className = "builder-row__iparam";
    row.appendChild(ref.input);

    var target = null;
    if (!reverse) {
      row.appendChild(chainLabel(":"));
      target = document.createElement("select");
      target.className = "builder-row__itarget";
      row.appendChild(target);
    }

    var iterate = document.createElement("button");
    iterate.type = "button";
    iterate.className = "builder-row__iterate";
    iterate.dataset.toggleIterate = "true";
    iterate.textContent = "↘ " + sections.dataset.msgIterate;
    iterate.setAttribute("aria-pressed", part.modifier === "iterate" ? "true" : "false");
    row.appendChild(iterate);

    var remove = document.createElement("button");
    remove.type = "button";
    remove.className = "builder-row__remove";
    remove.dataset.removeRow = "true";
    remove.setAttribute("aria-label", sections.dataset.msgRemove);
    remove.textContent = "×";
    row.appendChild(remove);

    function refill() {
      var t = type.input.value.trim();
      if (!t) return;
      fetchParams(t).then(function (meta) {
        var codes = Object.keys(meta)
          .filter(function (code) {
            var m = meta[code];
            if (m.type !== "reference") return false;
            /* A reverse include must be able to point back at the base. */
            if (!reverse) return true;
            return m.targets.length === 0 || m.targets.indexOf(base) >= 0;
          })
          .sort();
        fillList(ref.list, codes);
        if (target) {
          var chosen = bits[2] || "";
          var targets = ((meta[ref.input.value.trim()] || {}).targets || []).slice();
          target.textContent = "";
          option(target, "", sections.dataset.msgAnyTarget, chosen === "");
          targets.forEach(function (t2) {
            option(target, t2, t2, chosen === t2);
          });
          if (chosen && targets.indexOf(chosen) < 0) {
            option(target, chosen, chosen, true);
          }
        }
      });
    }
    type.input.addEventListener("input", refill);
    ref.input.addEventListener("input", refill);
    refill();

    return row;
  }

  /* Shows the drill-into affordance on condition rows whose parameter is a
   * reference, per the registry metadata for the current base type. */
  function refreshChainAffordances() {
    if (!sections) return;
    var meta = PARAM_META[sections.dataset.type || ""] || {};
    sections
      .querySelectorAll("#builder-conditions .builder-row")
      .forEach(function (row) {
        if (row.classList.contains("builder-row--chain")) return;
        if (row.classList.contains("builder-row--has")) return;
        var key = row.querySelector(".builder-row__key");
        var drill = row.querySelector("[data-chain-from]");
        if (!key || !drill) return;
        var m = meta[key.value.trim()];
        drill.hidden = !(m && m.type === "reference");
        regateModifiers(row);
      });
  }

  function builderRow(kind, part) {
    if (kind === "condition" && part.kind === "chain") return chainRow(part);
    if (kind === "condition" && part.kind === "has") return hasRow(part);
    if (kind === "include" && (!part.value || part.value.indexOf(":") > 0)) {
      return includeRow(part);
    }

    var row = document.createElement("div");
    row.className = "builder-row";

    var key;
    if (kind === "condition") {
      key = document.createElement("input");
      key.value = part.key;
      key.setAttribute("list", "param-options");
      key.placeholder = sections.dataset.msgParam;
      key.spellcheck = false;
    } else {
      key = document.createElement("select");
      var keys = kind === "include" ? INCLUDE_KEYS : CONTROL_KEYS;
      keys.forEach(function (k) {
        option(key, k, k, part.key === k);
      });
    }
    key.className = "builder-row__key";
    row.appendChild(key);

    if (kind === "condition") {
      /* Reference params can drill into their target (#394); hidden until
       * the registry metadata confirms the param is a reference. */
      var drill = document.createElement("button");
      drill.type = "button";
      drill.className = "builder-row__drill";
      drill.dataset.chainFrom = "true";
      drill.title = sections.dataset.msgChainInto;
      drill.textContent = "›";
      drill.hidden = true;
      row.appendChild(drill);
    }

    /* Comparator prefixes render in the modifier select but are rejoined
     * onto the value; colon modifiers are rejoined onto the key. */
    var value = part.value;
    var selectedMod = part.modifier;
    var prefix = PREFIX_RE.exec(value);
    if (!selectedMod && prefix) {
      selectedMod = prefix[1];
      value = value.slice(2);
    }

    if (kind !== "control") {
      var modifier = document.createElement("select");
      modifier.className = "builder-row__modifier";
      option(modifier, "", sections.dataset.msgMatchIs, !selectedMod);
      if (kind === "include") {
        option(modifier, "iterate", ":iterate", selectedMod === "iterate");
      } else {
        COLON_MODIFIERS.forEach(function (m) {
          option(modifier, m, ":" + m, selectedMod === m);
        });
        PREFIXES.forEach(function (p) {
          option(modifier, p, p, selectedMod === p);
        });
      }
      row.appendChild(modifier);
    }

    if (kind === "condition") {
      appendValues(row, value);
      var adv = document.createElement("button");
      adv.type = "button";
      adv.className = "builder-row__adv";
      adv.dataset.toggleMods = "true";
      adv.title = sections.dataset.msgModifyHeading;
      adv.setAttribute("aria-expanded", "false");
      adv.textContent = "⚙";
      row.appendChild(adv);
    } else {
      var valueInput = document.createElement("input");
      valueInput.className = "builder-row__value";
      valueInput.value = value;
      valueInput.placeholder = sections.dataset.msgValue;
      valueInput.spellcheck = false;
      row.appendChild(valueInput);
    }

    var remove = document.createElement("button");
    remove.type = "button";
    remove.className = "builder-row__remove";
    remove.dataset.removeRow = "true";
    remove.setAttribute("aria-label", sections.dataset.msgRemove);
    remove.textContent = "×";
    row.appendChild(remove);

    if (kind === "condition") {
      var panel = document.createElement("div");
      panel.className = "builder-row__modpanel";
      panel.hidden = true;
      row.appendChild(panel);
    }

    return row;
  }

  function builderHosts() {
    return {
      condition: document.getElementById("builder-conditions"),
      include: document.getElementById("builder-includes"),
      control: document.getElementById("builder-controls"),
    };
  }

  /* Marks the picker rail's active type. */
  function markRailType(type) {
    document
      .querySelectorAll("#type-rail-list a.filter-rail__item")
      .forEach(function (item) {
        if (item.dataset.type === type) {
          item.setAttribute("aria-current", "true");
        } else {
          item.removeAttribute("aria-current");
        }
      });
  }

  /* The last URL the rows themselves produced. A native `change` can fire on
   * the URL input after `updateUrl` rewrote it programmatically (the browser's
   * dirty-value flag survives), and rebuilding the rows mid-interaction would
   * yank focus and drop in-flight edits — so re-parsing skips its own echo. */
  var lastSerialized = null;

  /* URL → rows. */
  function renderBuilder() {
    if (!sections || !urlInput) return;
    if (urlInput.value === lastSerialized) return;
    var parsed = parseSearchUrl(urlInput.value);
    if (!parsed) {
      sections.hidden = true;
      markRailType(null);
      return;
    }
    sections.hidden = false;
    sections.dataset.type = parsed.type;
    markRailType(parsed.type);
    loadCatalog(parsed.type);

    var hosts = builderHosts();
    Object.keys(hosts).forEach(function (kind) {
      hosts[kind].textContent = "";
    });
    splitQuery(parsed.query).forEach(function (part) {
      var kind = bucketFor(part);
      hosts[kind].appendChild(builderRow(kind, part));
    });
    refreshChainAffordances();
    updatePlain();
  }

  /* Rows → URL. */
  function updateUrl() {
    if (!sections || !urlInput) return;
    var type = sections.dataset.type || "";
    var parts = [];
    sections.querySelectorAll(".builder-row").forEach(function (row) {
      var key;
      if (row.classList.contains("builder-row--chain")) {
        var segs = row.querySelectorAll(".builder-row__hopseg");
        var leaf = row.querySelector(".builder-row__cparam").value.trim();
        if (!segs.length || !leaf) return;
        var pieces = [];
        for (var si = 0; si < segs.length; si++) {
          var ref = segs[si].querySelector(".builder-row__chainref").value.trim();
          if (!ref) return;
          var ctype = segs[si].querySelector(".builder-row__ctype").value;
          pieces.push(ref + (ctype ? ":" + ctype : ""));
        }
        key = pieces.join(".") + "." + leaf;
      } else if (row.classList.contains("builder-row--include")) {
        var itype = row.querySelector(".builder-row__itype").value.trim();
        var iparam = row.querySelector(".builder-row__iparam").value.trim();
        if (!itype || !iparam) return;
        var itargetEl = row.querySelector(".builder-row__itarget");
        var itarget = itargetEl ? itargetEl.value : "";
        var iter = row.querySelector("[data-toggle-iterate]");
        key = row.dataset.includeKey;
        if (iter && iter.getAttribute("aria-pressed") === "true") key += ":iterate";
        parts.push(key + "=" + itype + ":" + iparam + (itarget ? ":" + itarget : ""));
        return;
      } else if (row.classList.contains("builder-row--has")) {
        var htype = row.querySelector(".builder-row__htype").value.trim();
        var href = row.querySelector(".builder-row__href").value.trim();
        var hleaf = row.querySelector(".builder-row__cparam").value.trim();
        if (!htype || !href || !hleaf) return;
        key = "_has:" + htype + ":" + href + ":" + hleaf;
      } else {
        key = row.querySelector(".builder-row__key").value.trim();
      }
      if (!key) return;
      var modifierEl = row.querySelector(".builder-row__modifier");
      var mod = modifierEl ? modifierEl.value : "";
      var isPrefix = PREFIXES.indexOf(mod) >= 0;
      var values = [];
      row.querySelectorAll(".builder-row__value").forEach(function (vi) {
        var v = vi.value.trim();
        if (!v) return;
        if (isPrefix && !PREFIX_RE.test(v)) v = mod + v;
        values.push(v);
      });
      var value = values.join(",");
      if (!isPrefix && mod) key += ":" + mod;
      parts.push(key + "=" + value);
    });
    urlInput.value =
      "GET /" + type + (parts.length ? "?" + parts.join("&") : "");
    lastSerialized = urlInput.value;
    updatePlain();
  }

  if (sections && urlInput) {
    urlInput.addEventListener("change", renderBuilder);
    sections.addEventListener("input", function (event) {
      if (event.target.closest(".builder-row")) {
        updateUrl();
        if (event.target.classList.contains("builder-row__key")) {
          refreshChainAffordances();
          regateModifiers(event.target.closest(".builder-row"));
        }
      }
    });
    sections.addEventListener("click", function (event) {
      var remove = event.target.closest("[data-remove-row]");
      var drillFrom = event.target.closest("[data-chain-from]");
      var toggleIterate = event.target.closest("[data-toggle-iterate]");
      var toggleMods = event.target.closest("[data-toggle-mods]");
      if (toggleIterate) {
        var on = toggleIterate.getAttribute("aria-pressed") === "true";
        toggleIterate.setAttribute("aria-pressed", on ? "false" : "true");
        updateUrl();
        return;
      }
      var modChip = event.target.closest("[data-mod-chip]");
      var addOr = event.target.closest("[data-add-or]");
      if (toggleMods) {
        var modRow = toggleMods.closest(".builder-row");
        var modPanel = modRow.querySelector(".builder-row__modpanel");
        regateModifiers(modRow);
        if (modPanel.hidden) {
          refreshModifierControls(modRow, modRow.dataset.modType || "");
        }
        modPanel.hidden = !modPanel.hidden;
        toggleMods.setAttribute("aria-expanded", modPanel.hidden ? "false" : "true");
        return;
      }
      if (modChip) {
        var chipRow = modChip.closest(".builder-row");
        var sel = chipRow.querySelector(".builder-row__modifier");
        sel.value = sel.value === modChip.dataset.modChip ? "" : modChip.dataset.modChip;
        fillModPanel(
          chipRow.querySelector(".builder-row__modpanel"),
          chipRow,
          applicableMods(chipRow.dataset.modType || ""),
        );
        updateUrl();
        return;
      }
      var removeOr = event.target.closest("[data-remove-or]");
      var add = event.target.closest("[data-add]");
      if (addOr) {
        var orHost = addOr.closest(".builder-row").querySelector(".builder-row__values");
        orValueInput(orHost, "").focus();
        return;
      }
      if (removeOr) {
        var orWrap = removeOr.closest(".builder-row__orvalue");
        if (orWrap.parentElement.children.length > 1) {
          orWrap.remove();
          updateUrl();
        }
        return;
      }
      if (remove) {
        remove.closest(".builder-row").remove();
        updateUrl();
      } else if (drillFrom) {
        /* Convert the condition row into a forward-chain row for its
         * reference param, keeping the value the user already typed. */
        var from = drillFrom.closest(".builder-row");
        var refParam = from.querySelector(".builder-row__key").value.trim();
        var kept = from.querySelector(".builder-row__value").value.trim();
        var chain = chainRow({
          kind: "chain",
          hops: [{ ref: refParam, type: "" }],
          key: "",
          modifier: "",
          value: kept,
        });
        from.replaceWith(chain);
        chain.querySelector(".builder-row__cparam").focus();
        updateUrl();
      } else if (add) {
        var kind = add.dataset.add;
        if (kind === "include-fwd" || kind === "include-rev") {
          var inc = includeRow({
            key: kind === "include-rev" ? "_revinclude" : "_include",
            modifier: "",
            value: "",
          });
          builderHosts().include.appendChild(inc);
          inc.querySelector(kind === "include-rev" ? ".builder-row__itype" : ".builder-row__iparam").focus();
          return;
        }
        if (kind === "has") {
          var has = hasRow({
            kind: "has",
            hasType: "",
            refParam: "",
            key: "",
            modifier: "",
            value: "",
          });
          builderHosts().condition.appendChild(has);
          has.querySelector(".builder-row__htype").focus();
          return;
        }
        var part = {
          key: kind === "include" ? "_include" : kind === "control" ? "_count" : "",
          modifier: "",
          value: "",
        };
        var row = builderRow(kind, part);
        builderHosts()[kind].appendChild(row);
        row.querySelector(kind === "condition" ? ".builder-row__key" : ".builder-row__value").focus();
      }
    });
  }

  /* ---- Resource picker rail --------------------------------------------
   * The type list, its links (`/ui/<page>?type=<name>`), and its counts are
   * all server-rendered (#541) from the shared `partials/type_rail.html`
   * macro. Without JavaScript the `<a>` navigates and the server marks the
   * selection; with it, a click is intercepted so the action happens
   * in-page and the URL still updates via `history.pushState`. */

  var railList = document.getElementById("type-rail-list");
  var railFilter = document.getElementById("type-rail-filter");

  /* Selects a resource type: syncs the rail, the builder, and the results —
   * the one path both the rail click and the initial page load (#605) drive,
   * so the two never drift apart.
   *
   * On the Resources page, `panel.dataset.selectedType` (the rail's
   * `<aside>`) is the single source of truth for "which type is selected";
   * this also keeps the "Create new" button's label in sync with it, from
   * the localized template the server put on `data-msg-create`. Both the
   * panel and the button are absent on the Saved Queries / Search pages,
   * where this rail only drives the search. */
  function selectType(type) {
    var panel = document.getElementById("resources");
    if (panel) panel.dataset.selectedType = type;
    var createBtn = document.getElementById("resource-create");
    if (createBtn && createBtn.dataset.msgCreate) {
      var label = createBtn.querySelector(".resources-create__label");
      if (label) label.textContent = createBtn.dataset.msgCreate.replace("{type}", type);
    }
    markRailType(type);
    urlInput.value = "GET /" + type;
    renderBuilder();
    runSearch("/" + encodeURIComponent(type), false);
  }

  if (railList) {
    railList.addEventListener("click", function (event) {
      var item = event.target.closest("a.filter-rail__item");
      if (!item || !urlInput) return;
      event.preventDefault();
      selectType(item.dataset.type);
      window.history.pushState({}, "", item.getAttribute("href"));
    });
  }
  if (railFilter && railList) {
    railFilter.addEventListener("input", function () {
      var needle = railFilter.value.trim().toLowerCase();
      railList
        .querySelectorAll("a.filter-rail__item[data-type]")
        .forEach(function (item) {
          item.hidden =
            !!needle && item.dataset.type.toLowerCase().indexOf(needle) < 0;
        });
    });
  }


  /* ---- "In plain English" (#395): a deterministic narration of the query,
   * assembled from server-rendered i18n fragments. No LLM involved — this is
   * the inverse companion of natural-language search. */
  var plainHost = document.getElementById("query-plain");
  var plainText = document.getElementById("query-plain-text");
  var PLAIN = null;
  (function () {
    var blob = document.getElementById("plain-english-msgs");
    if (blob) {
      try {
        PLAIN = JSON.parse(blob.textContent);
      } catch (e) {
        PLAIN = null;
      }
    }
  })();

  function tpl(text, args) {
    return text.replace(/\{(\w+)\}/g, function (_, k) {
      return args[k] != null ? args[k] : "";
    });
  }

  function plainValue(part) {
    var raw = part.value || "";
    var mod = part.modifier || "";
    if (mod === "missing" && (raw === "true" || raw === "false")) {
      return { verb: PLAIN.missing[raw], value: "", showValue: false };
    }
    var alternatives = raw.split(",").filter(Boolean).map(function (v) {
      var p = PREFIX_RE.exec(v);
      return p ? v.slice(2) : v;
    });
    var prefix = PREFIX_RE.exec(raw);
    var verbKey = mod || (prefix ? prefix[1] : "");
    var verb = PLAIN.verbs[verbKey] != null ? PLAIN.verbs[verbKey] : PLAIN.verbs[""];
    var joined = alternatives
      .map(function (v) {
        return "\u201C" + v + "\u201D";
      })
      .join(" " + PLAIN.or + " ");
    return { verb: verb, value: joined, showValue: joined !== "" };
  }

  function renderPlainClause(withValue, withoutValue, args, plain) {
    args.verb = plain.verb;
    if (plain.showValue) args.value = plain.value;
    return tpl(plain.showValue ? withValue : withoutValue, args);
  }

  function updatePlain() {
    if (!PLAIN || !plainHost || !urlInput) return;
    var parsed = parseSearchUrl(urlInput.value);
    if (!parsed) {
      plainHost.hidden = true;
      return;
    }
    var clauses = [];
    var extras = [];
    splitQuery(parsed.query).forEach(function (part) {
      if (part.kind === "chain") {
        var path = part.hops
          .map(function (h) {
            return h.ref + (h.type ? " (" + h.type + ")" : "");
          })
          .concat([part.key])
          .join(PLAIN.arrow + " ");
        var cv = plainValue(part);
        clauses.push(
          renderPlainClause(PLAIN.clause, PLAIN.clauseNoValue, { path: path }, cv),
        );
        return;
      }
      if (part.kind === "has") {
        var hv = plainValue(part);
        clauses.push(
          renderPlainClause(
            PLAIN.has,
            PLAIN.hasNoValue,
            { type: part.hasType, param: part.key },
            hv,
          ),
        );
        return;
      }
      if (part.key === "_include" || part.key === "_revinclude") {
        var bits = (part.value || "").split(":");
        var iter = part.modifier === "iterate" ? " " + PLAIN.iterate : "";
        if (part.key === "_include") {
          extras.push(
            tpl(PLAIN.include, {
              param: bits[1] || part.value,
              type: bits[0] || parsed.type,
              target: bits[2] ? " (" + bits[2] + ")" : "",
            }) + iter,
          );
        } else {
          extras.push(
            tpl(PLAIN.revinclude, { type: bits[0] || "?", param: bits[1] || "?" }) + iter,
          );
        }
        return;
      }
      if (part.key === "_count") {
        extras.push(tpl(PLAIN.count, { n: part.value }));
        return;
      }
      if (part.key === "_sort") {
        extras.push(tpl(PLAIN.sort, { sort: part.value }));
        return;
      }
      if (CONTROL_KEYS.indexOf(part.key) >= 0 || !part.key) return;
      var v = plainValue(part);
      clauses.push(
        renderPlainClause(PLAIN.clause, PLAIN.clauseNoValue, { path: part.key }, v),
      );
    });

    var sentence = tpl(PLAIN.find, { type: parsed.type });
    if (clauses.length) {
      sentence += " \u2014 " + clauses.join(" " + PLAIN.and + " ");
    }
    if (extras.length) sentence += ". " + extras.join("; ");
    sentence += ".";
    plainText.textContent = sentence;
    plainHost.hidden = false;
  }

  /* ---- Results: the FHIR search response, rendered in-page ------------- */

  /* The last path handed to runSearch — what a data-changed re-run repeats. */
  var lastSearchPath = null;

  var results = {
    card: document.getElementById("query-results"),
    head: document.getElementById("query-results-head"),
    body: document.getElementById("query-results-body"),
    meta: document.getElementById("query-results-meta"),
    note: document.getElementById("query-results-note"),
    open: document.getElementById("query-results-open"),
    prev: document.getElementById("query-results-prev"),
    next: document.getElementById("query-results-next"),
    sort: document.getElementById("query-results-sort"),
  };

  /* Compact display heuristics for common FHIR shapes (HumanName,
   * CodeableConcept, Reference, Quantity); everything else is truncated
   * JSON rather than a blank cell. */
  function fmt(value) {
    if (value == null) return "";
    if (typeof value !== "object") return String(value);
    if (Array.isArray(value)) {
      if (!value.length) return "";
      var first = fmt(value[0]);
      return value.length > 1 ? first + " +" + (value.length - 1) : first;
    }
    if (value.family || value.given)
      return [value.family, (value.given || []).join(" ")]
        .filter(Boolean)
        .join(", ");
    if (value.text) return value.text;
    if (value.coding) return fmt(value.coding);
    if (value.display) return value.display;
    if (value.reference) return value.reference;
    if (value.value !== undefined && value.unit)
      return value.value + " " + value.unit;
    if (value.code) return value.code;
    var json = JSON.stringify(value);
    return json.length > 60 ? json.slice(0, 60) + "…" : json;
  }

  /* Typed default columns (#416): common fields per resource type when the
   * query names no _elements; unknown types keep the compact id/updated view. */
  var DEFAULT_COLUMNS = {
    Patient: ["name", "gender", "birthDate", "managingOrganization"],
    Practitioner: ["name", "gender"],
    Organization: ["name", "type"],
    Observation: ["code", "status", "subject"],
    Encounter: ["status", "class", "subject"],
    Condition: ["code", "clinicalStatus", "subject"],
  };

  function elementColumns(query) {
    var columns = [];
    splitQuery(query).forEach(function (part) {
      if (part.key !== "_elements") return;
      part.value.split(",").forEach(function (el) {
        el = el.trim();
        if (el && el !== "id" && columns.indexOf(el) < 0) columns.push(el);
      });
    });
    return columns;
  }

  function cell(row, text, mono) {
    var td = document.createElement("td");
    if (mono) {
      var span = document.createElement("span");
      span.className = "url";
      span.textContent = text;
      td.appendChild(span);
    } else {
      td.textContent = text;
    }
    row.appendChild(td);
    return td;
  }

  function pagerLink(bundle, relation) {
    var links = (bundle && bundle.link) || [];
    for (var i = 0; i < links.length; i++) {
      if (links[i].relation === relation && links[i].url) return links[i].url;
    }
    return null;
  }

  function renderResults(path, ok, body) {
    var card = results.card;
    if (!card) return;
    card.hidden = false;
    results.open.href = path;
    results.head.textContent = "";
    results.body.textContent = "";
    results.meta.textContent = "";
    results.note.textContent = "";
    results.prev.hidden = true;
    results.next.hidden = true;

    if (!ok || !body || body.resourceType !== "Bundle") {
      var diagnostics =
        body &&
        body.issue &&
        body.issue[0] &&
        (body.issue[0].diagnostics ||
          (body.issue[0].details && body.issue[0].details.text));
      results.note.textContent = diagnostics || messages.msgError;
      return;
    }

    var parsed = parseSearchUrl(path) || { type: "", query: "" };
    var entries = body.entry || [];
    var primary = entries.filter(function (entry) {
      return (
        entry.resource && entry.resource.resourceType === parsed.type
      );
    });
    var included = entries.length - primary.length;

    var total =
      typeof body.total === "number" ? body.total : primary.length;
    var meta = card.dataset.msgTotal.replace("{count}", total);
    if (included > 0)
      meta += " · " + card.dataset.msgIncluded.replace("{count}", included);
    results.meta.textContent = meta;

    var columns = elementColumns(parsed.query);
    if (!columns.length) columns = DEFAULT_COLUMNS[parsed.type] || [];
    var headRow = document.createElement("tr");
    var th = document.createElement("th");
    th.textContent = "id";
    headRow.appendChild(th);
    columns.forEach(function (col) {
      var cellEl = document.createElement("th");
      cellEl.textContent = col;
      headRow.appendChild(cellEl);
    });
    var thUpdated = document.createElement("th");
    thUpdated.textContent = card.dataset.msgUpdated;
    headRow.appendChild(thUpdated);
    results.head.appendChild(headRow);

    primary.forEach(function (entry) {
      var resource = entry.resource;
      var row = document.createElement("tr");
      var idCell = document.createElement("td");
      var link = document.createElement("a");
      link.className = "url";
      link.href = "/" + parsed.type + "/" + resource.id;
      link.target = "_blank";
      link.rel = "noopener";
      link.textContent = resource.id || "";
      idCell.appendChild(link);
      row.appendChild(idCell);
      columns.forEach(function (col) {
        cell(row, fmt(resource[col]));
      });
      cell(
        row,
        resource.meta && resource.meta.lastUpdated
          ? whenText(resource.meta.lastUpdated)
          : ""
      );
      results.body.appendChild(row);
    });

    if (!primary.length) results.note.textContent = card.dataset.msgEmpty;

    if (results.sort) {
      var sortValue = "";
      splitQuery(parsed.query).forEach(function (part) {
        if (part.key === "_sort") sortValue = part.value;
      });
      results.sort.value = sortValue;
      if (results.sort.value !== sortValue) results.sort.value = "";
    }

    var prevUrl = pagerLink(body, "previous");
    var nextUrl = pagerLink(body, "next");
    if (prevUrl) {
      results.prev.hidden = false;
      results.prev.dataset.url = prevUrl;
    }
    if (nextUrl) {
      results.next.hidden = false;
      results.next.dataset.url = nextUrl;
    }
  }

  /* Runs a search against the FHIR API and renders the Bundle in-page.
   * `record` adds it to the roaming recent list (explicit runs only, so
   * paging does not spam recents). */
  function runSearch(path, record) {
    lastSearchPath = path;
    if (!results.card) {
      window.open(path, "_blank", "noopener");
    } else {
      fetch(path, {
        headers: fhirHeaders(),
        credentials: "same-origin",
      })
        .then(function (response) {
          return response
            .json()
            .catch(function () {
              return null;
            })
            .then(function (body) {
              renderResults(path, response.ok, body);
            });
        })
        .catch(function () {
          renderResults(path, false, null);
        });
    }
    if (record) recordRecent(path);
  }

  results.card &&
    results.card.addEventListener("click", function (event) {
      var pager = event.target.closest("button[data-url]");
      if (pager) runSearch(pager.dataset.url, false);
    });

  /* Sort re-runs the current results path with the picked `_sort` (#416). */
  results.sort &&
    results.sort.addEventListener("change", function () {
      var current = results.open && results.open.getAttribute("href");
      if (!current) return;
      var parsed = parseSearchUrl(current);
      if (!parsed) return;
      var parts = (parsed.query || "").split("&").filter(function (p) {
        return p && p.indexOf("_sort=") !== 0;
      });
      if (results.sort.value) parts.push("_sort=" + results.sort.value);
      runSearch(searchPath(parsed.type, parts.join("&")), false);
    });

  /* ---- Recent searches & the saved list -------------------------------- */

  /* Last-accessed first; never-run entries follow, newest created first. */
  function compareEntries(a, b) {
    var aRun = a.entry.lastAccessedAt || "";
    var bRun = b.entry.lastAccessedAt || "";
    if (aRun !== bRun) return aRun > bRun ? -1 : 1;
    var aCreated = a.entry.createdAt || "";
    var bCreated = b.entry.createdAt || "";
    if (aCreated !== bCreated) return aCreated > bCreated ? -1 : 1;
    return a.id < b.id ? -1 : 1;
  }

  function whenText(iso) {
    var when = new Date(iso);
    if (isNaN(when.getTime())) return iso || "";
    var now = new Date();
    return when.toDateString() === now.toDateString()
      ? when.toLocaleTimeString(lang, { hour: "2-digit", minute: "2-digit" })
      : when.toLocaleDateString(lang);
  }

  function metaText(entry) {
    if (!entry.lastAccessedAt) return messages.msgNeverRun;
    var when = new Date(entry.lastAccessedAt);
    var text = isNaN(when.getTime())
      ? entry.lastAccessedAt
      : when.toLocaleString(lang);
    var runs = Number(entry.accessCount);
    return runs > 0 ? text + " · " + runs + "×" : text;
  }

  function button(label, action, resourceType, id) {
    var el = document.createElement("button");
    el.type = "button";
    el.className = "btn";
    el.textContent = label;
    el.dataset.action = action;
    el.dataset.type = resourceType;
    el.dataset.id = id;
    return el;
  }

  function renderRecent(doc) {
    if (!recentHost) return;
    recentHost.textContent = "";

    // Saved (named) queries come first — the same document, surfaced in the
    // one dropdown so Resources has both saved and recent in reach (#282).
    var byType = savedQueries(doc);
    var savedRows = [];
    Object.keys(byType)
      .sort()
      .forEach(function (type) {
        var entries = byType[type] || {};
        Object.keys(entries).forEach(function (id) {
          var entry = entries[id] || {};
          if (entry.query === undefined) return;
          savedRows.push({ type: type, name: entry.name || id, query: entry.query });
        });
      });

    if (savedRows.length) {
      var heading = document.createElement("div");
      heading.className = "recent-group";
      heading.textContent = recentHost.dataset.msgSaved || "Saved";
      recentHost.appendChild(heading);
      savedRows.forEach(function (row) {
        var path = searchPath(row.type, row.query);
        var btn = document.createElement("button");
        btn.type = "button";
        btn.className = "recent-item__query recent-item__saved";
        btn.dataset.savedLoad = path;
        btn.textContent = row.name;
        btn.title = "GET " + path;
        recentHost.appendChild(btn);
      });
      var recentHeading = document.createElement("div");
      recentHeading.className = "recent-group";
      recentHeading.textContent = recentHost.dataset.msgRecentGroup || "Recent";
      recentHost.appendChild(recentHeading);
    }

    var list = recentSearches(doc);

    if (!list.length) {
      var empty = document.createElement("p");
      empty.className = "recent-empty";
      empty.textContent = recentHost.dataset.msgEmpty;
      recentHost.appendChild(empty);
      return;
    }

    list.forEach(function (item, index) {
      var row = document.createElement("div");
      row.className = "recent-item";

      var load = document.createElement("button");
      load.type = "button";
      load.className = "recent-item__query";
      load.dataset.recentLoad = String(index);
      load.textContent = "GET " + item.query;
      load.title = "GET " + item.query;

      var when = document.createElement("span");
      when.className = "recent-item__when";
      when.textContent = whenText(item.at);

      var del = document.createElement("button");
      del.type = "button";
      del.className = "recent-item__del";
      del.dataset.recentDelete = String(index);
      del.setAttribute("aria-label", recentHost.dataset.msgDelete);
      del.textContent = "×";

      row.appendChild(load);
      row.appendChild(when);
      row.appendChild(del);
      recentHost.appendChild(row);
    });
  }

  /* Prepends a run to recentSearches: dedupe by query, newest first, capped.
   * Reads fresh state first so runs from other tabs are not clobbered. */
  function recordRecent(path) {
    return fetchDocument().then(function (doc) {
      var list = recentSearches(doc).filter(function (item) {
        return item.query !== path;
      });
      list.unshift({ query: path, at: new Date().toISOString() });
      return patchDocument(
        { recentSearches: list.slice(0, MAX_RECENT) },
        0
      ).then(render);
    });
  }

  function render(doc) {
    clearError();
    renderRecent(doc);
    /* The Search page renders the builder and the recent list but no saved
     * list; there is nothing further to draw there. */
    if (!root) return;

    var byType = savedQueries(doc);
    root.textContent = "";

    var types = Object.keys(byType).sort();
    var total = 0;

    types.forEach(function (resourceType) {
      var entries = byType[resourceType];
      if (!entries || typeof entries !== "object" || Array.isArray(entries))
        return;
      var rows = Object.keys(entries)
        .map(function (id) {
          return { id: id, entry: entries[id] || {} };
        })
        .sort(compareEntries);
      if (!rows.length) return;
      total += rows.length;

      var group = document.createElement("section");
      group.className = "card query-group";
      var heading = document.createElement("h2");
      heading.className = "query-group__type";
      heading.textContent = resourceType;
      group.appendChild(heading);

      var list = document.createElement("ul");
      list.className = "query-list";
      rows.forEach(function (row) {
        var item = document.createElement("li");
        item.className = "query-row";

        var main = document.createElement("div");
        main.className = "query-row__main";
        var name = document.createElement("span");
        name.className = "query-row__name";
        name.textContent = row.entry.name || row.id;
        var query = document.createElement("code");
        query.className = "query-row__query";
        query.textContent = row.entry.query || "";
        main.appendChild(name);
        main.appendChild(query);

        var meta = document.createElement("span");
        meta.className = "query-row__meta";
        meta.textContent = metaText(row.entry);

        var actions = document.createElement("div");
        actions.className = "query-row__actions";
        actions.appendChild(
          button(messages.msgRun, "run", resourceType, row.id)
        );
        actions.appendChild(
          button(messages.msgRename, "rename", resourceType, row.id)
        );
        actions.appendChild(
          button(messages.msgDelete, "delete", resourceType, row.id)
        );

        item.appendChild(main);
        item.appendChild(meta);
        item.appendChild(actions);
        list.appendChild(item);
      });
      group.appendChild(list);
      root.appendChild(group);
    });

    if (!total) {
      var empty = document.createElement("p");
      empty.className = "query-empty";
      empty.textContent = messages.msgEmpty;
      root.appendChild(empty);
    }
  }

  /* Errors surface next to the query strip — the control they are almost
   * always about — on both pages that render the builder. */
  function showError(outcome, fallback) {
    if (!errorHost) return;
    errorHost.textContent =
      (outcome &&
        outcome.issue &&
        outcome.issue[0] &&
        outcome.issue[0].diagnostics) ||
      fallback ||
      messages.msgError;
    errorHost.hidden = false;
  }

  function clearError() {
    if (!errorHost) return;
    errorHost.textContent = "";
    errorHost.hidden = true;
  }

  function reload() {
    return fetchDocument()
      .then(render)
      .catch(function (failure) {
        var unavailable = failure && failure.unavailable;
        /* Saved queries need the per-user settings document; search does not.
         * Without the list, the page is the Search page — leave its form
         * alone and say nothing. */
        if (!root) return;
        root.textContent = "";
        var note = document.createElement("p");
        note.className = "query-empty";
        note.textContent = unavailable
          ? messages.msgUnavailable
          : messages.msgError;
        root.appendChild(note);
        if (unavailable && form) form.hidden = true;
      });
  }

  function mutate(resourceType, id, entry) {
    return patchEntry(resourceType, id, entry)
      .then(function (doc) {
        render(doc);
        return true;
      })
      .catch(function (failure) {
        return reload().then(function () {
          showError(failure && failure.outcome);
          return false;
        });
      });
  }

  function entryFor(doc, resourceType, id) {
    var entries = savedQueries(doc)[resourceType];
    return (entries && entries[id]) || null;
  }

  /* Loads a query into the builder without running it. */
  function loadIntoBuilder(path) {
    if (!urlInput) return;
    urlInput.value = "GET " + path;
    renderBuilder();
  }

  /* Copy the query exactly as shown: what gets copied is what would run. */
  var copyButton = document.getElementById("query-copy");
  if (copyButton && navigator.clipboard) {
    copyButton.addEventListener("click", function () {
      navigator.clipboard.writeText(urlInput.value || "");
    });
  } else if (copyButton) {
    copyButton.hidden = true;
  }

  /* Saved-list actions — only the Saved Queries page renders the list. */
  if (root) {
    root.addEventListener("click", function (event) {
      var target = event.target.closest("button[data-action]");
      if (!target) return;
      var resourceType = target.dataset.type;
      var id = target.dataset.id;

      fetchDocument().then(function (doc) {
        var entry = entryFor(doc, resourceType, id);
        if (!entry) {
          render(doc);
          return;
        }

        if (target.dataset.action === "run") {
          var path = searchPath(resourceType, entry.query || "");
          loadIntoBuilder(path);
          runSearch(path, true);
          mutate(resourceType, id, {
            lastAccessedAt: new Date().toISOString(),
            accessCount: (Number(entry.accessCount) || 0) + 1,
          });
        } else if (target.dataset.action === "rename") {
          var name = window.prompt(messages.msgRenamePrompt, entry.name || "");
          if (name === null) return;
          name = name.trim();
          if (!name || name === entry.name) return;
          mutate(resourceType, id, { name: name });
        } else if (target.dataset.action === "delete") {
          var label = (entry.name || id).toString();
          if (
            !window.confirm(messages.msgConfirmDelete.replace("{name}", label))
          )
            return;
          mutate(resourceType, id, null);
        }
      });
    });
  }

  if (recentHost) {
    recentHost.addEventListener("click", function (event) {
      // A saved query carries its full path — load it straight into the builder.
      var saved = event.target.closest("[data-saved-load]");
      if (saved) {
        loadIntoBuilder(saved.dataset.savedLoad);
        var box = recentHost.closest("details");
        if (box) box.open = false;
        if (urlInput) urlInput.focus();
        return;
      }
      var load = event.target.closest("[data-recent-load]");
      var del = event.target.closest("[data-recent-delete]");
      if (!load && !del) return;

      fetchDocument().then(function (doc) {
        var list = recentSearches(doc);
        if (load) {
          var item = list[Number(load.dataset.recentLoad)];
          if (item) {
            loadIntoBuilder(item.query);
            var dd = recentHost.closest("details");
            if (dd) dd.open = false;
            if (urlInput) urlInput.focus();
          }
          return;
        }
        list.splice(Number(del.dataset.recentDelete), 1);
        patchDocument({ recentSearches: list }, 0).then(render);
      });
    });
  }

  if (form) {
    form.addEventListener("submit", function (event) {
      event.preventDefault();
      var intent =
        (event.submitter && event.submitter.dataset.intent) || "run";
      var parsed = parseSearchUrl(form.elements.url.value);
      if (!parsed) {
        reload().then(function () {
          showError(null, messages.msgInvalidUrl);
        });
        return;
      }

      if (intent === "run") {
        runSearch(searchPath(parsed.type, parsed.query), true);
        return;
      }

      var name = form.elements.name.value.trim();
      if (!name) {
        form.elements.name.focus();
        return;
      }
      var id =
        Date.now().toString(36) +
        "-" +
        Math.random().toString(36).slice(2, 8);
      mutate(parsed.type, id, {
        name: name,
        query: parsed.query,
        createdAt: new Date().toISOString(),
      }).then(function (saved) {
        if (!saved) return;
        form.elements.name.value = "";
      });
    });
  }

  /* Data changed behind the results (a save or delete in the Resources
   * modal announces it): re-run the last search so the table shows what is
   * actually stored, without recording a new recent. The rail's counts are
   * server-rendered (#541) and catch up on the next full page load. */
  document.addEventListener("hfs:data-changed", function () {
    if (lastSearchPath) runSearch(lastSearchPath, false);
  });

  reload();

  /* Deep link: /ui/queries?url=/Patient?name=smith loads the builder and
   * runs immediately — also what saved/recent entries could link to. */
  var deepLink = new URLSearchParams(window.location.search).get("url");
  if (deepLink && urlInput) {
    loadIntoBuilder(deepLink.replace(/^GET\s+/i, ""));
    var parsedDeep = parseSearchUrl(urlInput.value);
    if (parsedDeep)
      runSearch(searchPath(parsedDeep.type, parsedDeep.query), true);
  } else {
    // Resources opens in Patient (or `?type=`) context (#605): the same
    // path as a rail click, so the builder and results match the type the
    // server already selected — without registering a "recently used" entry,
    // since that only fires on an actual rail click (resource-filter.js).
    var initialPanel = document.getElementById("resources");
    if (initialPanel && urlInput) {
      selectType(initialPanel.dataset.selectedType || "Patient");
    } else {
      renderBuilder();
    }
  }
})();
