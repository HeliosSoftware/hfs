/*
 * ViewDefinition editor mount (#753 evaluation POC; not merged upstream).
 *
 * Progressive enhancement over the plain `<textarea class="json-editor"
 * name="json">` in `#vd-editor-form` on `/ui/sql/view-definitions`: if the
 * CodeMirror 6 bundle (ticket 01, `/ui/assets/vendor/codemirror.bundle.js`,
 * global `window.HfsCodeMirror`) loaded and that textarea exists, this
 * mounts a CodeMirror 6 editor on top of it with two language layers -
 * JSON on the outside, FHIRPath (the `lezer-fhirpath` grammar) injected
 * into the string values of the properties that hold FHIRPath expressions.
 * The textarea stays in the DOM (hidden, not removed) and stays the form's
 * source of truth: every doc change is written straight back into
 * `textarea.value`, so Save and Duplicate - plain POSTs of this form - keep
 * submitting exactly what the editor shows, with or without this script.
 *
 * Without the bundle, without JS, or if anything below throws, this file
 * does nothing (or backs out cleanly) and the page is the textarea as it
 * is today. Diagnostics, completion, and i18n are out of scope for this
 * ticket - see the epic doc.
 */
(function () {
  "use strict";

  var CM = window.HfsCodeMirror;
  var form = document.getElementById("vd-editor-form");
  var textarea = form ? form.querySelector('textarea[name="json"]') : null;
  if (!CM || !textarea) return;

  /* ---- FHIRPath injection: which JSON string values are expressions ----
   *
   * A string is a FHIRPath expression iff its *immediate* parent Property
   * is named "path", "forEach", or "forEachOrNull", or it is an element of
   * an Array whose owning Property is named "repeat" - matched locally, so
   * it fires at any nesting depth without walking the full ancestor chain
   * (select[].column[].path, select[].select[]..., where[].path, and
   * unionAll all resolve through the same two checks). name/description/
   * resource/status/url/title and everything else never match, because
   * their parent Property's name is never one of the four above.
   */
  var EXPRESSION_PROPERTIES = { path: true, forEach: true, forEachOrNull: true };

  function propertyKey(propertyNameNode, input) {
    var raw = input.read(propertyNameNode.from, propertyNameNode.to);
    return raw.length >= 2 && raw.charAt(0) === '"' && raw.charAt(raw.length - 1) === '"'
      ? raw.slice(1, -1)
      : raw;
  }

  var fhirpathLanguage = CM.LRLanguage.define({ parser: CM.fhirpath.parser });

  function nestFhirpath(node, input) {
    if (node.name !== "String") return null;

    var parent = node.node.parent;
    if (!parent) return null;
    var property = null;
    var mustBeRepeat = false;
    if (parent.name === "Property") {
      property = parent;
    } else if (parent.name === "Array" && parent.parent && parent.parent.name === "Property") {
      property = parent.parent;
      mustBeRepeat = true;
    }
    if (!property) return null;

    var nameNode = property.firstChild;
    if (!nameNode || nameNode.name !== "PropertyName") return null;
    var key = propertyKey(nameNode, input);
    var matches = mustBeRepeat ? key === "repeat" : EXPRESSION_PROPERTIES.hasOwnProperty(key);
    if (!matches) return null;

    // Overlay the string's content, excluding the surrounding quotes.
    var from = node.from + 1;
    var to = node.to - 1;
    // "": nothing to parse, and an empty overlay range would throw inside
    // @lezer/common's parseMixed.
    if (to <= from) return null;

    // Escaped quotes/backslashes inside the literal are not unescaped
    // before parsing (RF5 does not require it); the FHIRPath grammar just
    // fails to parse cleanly there, which is fine - the string still
    // renders, only without FHIRPath coloring: it degrades to a plain
    // JSON string.
    if (input.read(from, to).indexOf("\\") !== -1) return null;

    return { parser: fhirpathLanguage.parser, overlay: [{ from: from, to: to }] };
  }

  var jsonWithFhirpath = new CM.LanguageSupport(
    CM.jsonLanguage.configure({ wrap: CM.parseMixed(nestFhirpath) })
  );

  /* ---- Highlighting: two HighlightStyles, each scoped to its own
   * language, so the same generic tags (tags.string, tags.number, ...)
   * that both the JSON grammar and lezer-fhirpath happen to use can still
   * be colored differently depending on which side of the injection they
   * came from. Classes only - every color lives in app.css as a CSS
   * variable, never a fixed value from here (RF6). */
  var jsonHighlightStyle = CM.HighlightStyle.define(
    [
      { tag: CM.tags.propertyName, class: "cmt-json-key" },
      { tag: CM.tags.string, class: "cmt-json-string" },
      { tag: CM.tags.number, class: "cmt-json-number" },
      { tag: [CM.tags.bool, CM.tags.null], class: "cmt-json-literal" },
      { tag: [CM.tags.separator, CM.tags.squareBracket, CM.tags.brace], class: "cmt-json-punct" },
    ],
    { scope: CM.jsonLanguage }
  );

  var fhirpathHighlightStyle = CM.HighlightStyle.define(
    [
      {
        tag: [CM.tags.variableName, CM.tags.special(CM.tags.variableName), CM.tags.typeName],
        class: "cmt-fp-path",
      },
      {
        tag: [CM.tags.function(CM.tags.variableName), CM.tags.function(CM.tags.attributeName)],
        class: "cmt-fp-function",
      },
      { tag: CM.tags.keyword, class: "cmt-fp-keyword" },
      // No dedicated FHIRPath punctuation variable (RF6 lists 7 --fp-*
      // tokens, not 8): parens/brackets/commas read naturally as part of
      // "operators" here.
      {
        tag: [CM.tags.operator, CM.tags.paren, CM.tags.squareBracket, CM.tags.separator],
        class: "cmt-fp-operator",
      },
      {
        tag: [
          CM.tags.string,
          CM.tags.number,
          CM.tags.bool,
          CM.tags.literal,
          CM.tags.special(CM.tags.literal),
        ],
        class: "cmt-fp-literal",
      },
      { tag: CM.tags.constant(CM.tags.variableName), class: "cmt-fp-constant" },
      { tag: CM.tags.comment, class: "cmt-fp-comment" },
    ],
    { scope: fhirpathLanguage }
  );

  /* ---- Server lint (#753 ticket 03) -----------------------------------
   *
   * The browser only knows syntax; the server knows FHIR. A local
   * jsonParseLinter() pass runs first and, if the JSON itself does not
   * parse, is all that shows - the server is not called for text it
   * cannot even read as JSON. Once the JSON parses, POST it to
   * /ui/sql/view-definitions/lint and translate its {pointer, span}
   * diagnostics into CM6 {from, to} ranges by walking the *browser's own*
   * syntax tree - the server only ever speaks in JSON pointers, since it
   * has no notion of "line 4, column 12" for a document it never parsed
   * into a CM6 tree.
   *
   * CM6's own lint plugin already discards a stale async result on its
   * own (it compares the doc captured when a lint run started against the
   * live doc before dispatching), so nothing here needs its own
   * generation counter for that.
   */

  function unescapePointerSegment(segment) {
    return segment.replace(/~1/g, "/").replace(/~0/g, "~");
  }

  function pointerSegments(pointer) {
    return pointer === "" ? [] : pointer.slice(1).split("/").map(unescapePointerSegment);
  }

  function propertyKeyText(propertyNameNode, doc) {
    var raw = doc.sliceString(propertyNameNode.from, propertyNameNode.to);
    return raw.length >= 2 && raw.charAt(0) === '"' && raw.charAt(raw.length - 1) === '"'
      ? raw.slice(1, -1)
      : raw;
  }

  /* The Property node inside `objectNode` whose PropertyName reads `key`. */
  function findProperty(objectNode, key, doc) {
    for (var child = objectNode.firstChild; child; child = child.nextSibling) {
      if (child.name !== "Property") continue;
      var nameNode = child.firstChild;
      if (nameNode && nameNode.name === "PropertyName" && propertyKeyText(nameNode, doc) === key) {
        return child;
      }
    }
    return null;
  }

  /* A Property's value is its last child (PropertyName and ":" come first). */
  function propertyValue(propertyNode) {
    var last = propertyNode.lastChild;
    return last && last.name !== ":" ? last : null;
  }

  function arrayItemAt(arrayNode, index) {
    var i = 0;
    for (var child = arrayNode.firstChild; child; child = child.nextSibling) {
      if (child.name === "[" || child.name === "]" || child.name === ",") continue;
      if (i === index) return child;
      i++;
    }
    return null;
  }

  /* Walks the JSON syntax tree from its root value, following `pointer`'s
   * segments (an object key by name, an array index numerically), and
   * returns the node at that position - or null if the pointer does not
   * resolve against this document's actual shape (RF8: painted on line 1
   * by the caller in that case). */
  function resolveByPointer(tree, doc, pointer) {
    var node = tree.topNode.getChild("Object") || tree.topNode.firstChild;
    if (!node) return null;
    var segments = pointerSegments(pointer);
    for (var i = 0; i < segments.length; i++) {
      var segment = segments[i];
      if (node.name === "Object") {
        var property = findProperty(node, segment, doc);
        if (!property) return null;
        node = propertyValue(property);
      } else if (node.name === "Array") {
        if (!/^\d+$/.test(segment)) return null;
        node = arrayItemAt(node, Number(segment));
      } else {
        return null;
      }
      if (!node) return null;
    }
    return node;
  }

  function fallbackRange(doc) {
    var line = doc.line(1);
    return { from: line.from, to: line.to };
  }

  /* RF8: for an Object/Array (which may span many lines), just the first
   * line - from its opening "{"/"[" to wherever that line ends. A scalar
   * value is already within one line, so its own range is used as-is. */
  function valueRange(doc, node) {
    if (node.name === "Object" || node.name === "Array") {
      var line = doc.lineAt(node.from);
      return { from: node.from, to: Math.min(line.to, node.to) };
    }
    return { from: node.from, to: node.to };
  }

  /* UnknownKey locates the *key*, not the value: resolve everything but the
   * pointer's last segment to find the containing object, then find that
   * segment's own PropertyName inside it. */
  function unknownKeyRange(tree, doc, pointer) {
    var cut = pointer.lastIndexOf("/");
    var parent = resolveByPointer(tree, doc, pointer.slice(0, cut));
    if (!parent || parent.name !== "Object") return null;
    var property = findProperty(parent, unescapePointerSegment(pointer.slice(cut + 1)), doc);
    var nameNode = property ? property.firstChild : null;
    return nameNode ? { from: nameNode.from, to: nameNode.to } : null;
  }

  /* FhirPathSyntax/UndeclaredConstant: the span is a char offset into the
   * *content* of the pointed-at string (excluding its quotes). Escapes are
   * not unescaped anywhere in this ticket's pipeline (server or client), so
   * a span computed against unescaped text would misalign the moment the
   * string contains one - degrade to underlining the whole string instead,
   * exactly like the server's own FhirPathSyntax message already does when
   * it detects an escape (see lint.rs). */
  function spanRange(doc, node, span) {
    if (node.name !== "String" || !span) return { from: node.from, to: node.to };
    var contentFrom = node.from + 1;
    var contentTo = node.to - 1;
    if (doc.sliceString(contentFrom, contentTo).indexOf("\\") !== -1) {
      return { from: node.from, to: node.to };
    }
    var from = contentFrom + span.start;
    var to = contentFrom + span.end;
    return from >= node.from && to <= node.to && from <= to
      ? { from: from, to: to }
      : { from: node.from, to: node.to };
  }

  function diagnosticRange(view, diagnostic) {
    var tree = CM.syntaxTree(view.state);
    var doc = view.state.doc;

    if (diagnostic.code === "unknown-key") {
      return unknownKeyRange(tree, doc, diagnostic.pointer) || fallbackRange(doc);
    }

    var node = resolveByPointer(tree, doc, diagnostic.pointer);
    if (!node) return fallbackRange(doc);

    if (diagnostic.code === "fhirpath-syntax" || diagnostic.code === "undeclared-constant") {
      return spanRange(doc, node, diagnostic.span);
    }
    // missing-required, wrong-type, empty-required, duplicate-column-name,
    // select-without-output, multiple-iteration-directives,
    // not-a-view-definition, and anything this POC does not yet know about.
    return valueRange(doc, node);
  }

  function toCmDiagnostic(view, serverDiagnostic) {
    var docLength = view.state.doc.length;
    var range = diagnosticRange(view, serverDiagnostic);
    var from = Math.max(0, Math.min(range.from, docLength));
    var to = Math.max(from, Math.min(range.to, docLength));
    return {
      from: from,
      to: to,
      severity: serverDiagnostic.severity === "warning" ? "warning" : "error",
      message: serverDiagnostic.message,
    };
  }

  function fetchServerDiagnostics(view) {
    return fetch("/ui/sql/view-definitions/lint", {
      method: "POST",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json" },
      body: view.state.doc.toString(),
    })
      .then(function (response) {
        if (!response.ok) throw new Error("lint endpoint status " + response.status);
        return response.json();
      })
      .then(function (data) {
        var items = data && Array.isArray(data.diagnostics) ? data.diagnostics : [];
        return items.map(function (d) {
          return toCmDiagnostic(view, d);
        });
      })
      .catch(function (error) {
        // RF7: network failures, 5xx, 401 - never a visible or console.error
        // failure. Local diagnostics (if any) keep working regardless.
        if (window.console && console.debug) console.debug("vd-editor: server lint unavailable", error);
        return [];
      });
  }

  var jsonSyntaxLinter = CM.jsonParseLinter();

  function vdLinter(view) {
    var syntaxErrors = jsonSyntaxLinter(view);
    return syntaxErrors.length > 0 ? syntaxErrors : fetchServerDiagnostics(view);
  }

  /* ---- Mount --------------------------------------------------------- */

  function mount() {
    var ariaLabel = textarea.getAttribute("aria-label") || "";

    // Build the wrapper and the EditorView fully in memory first, and only
    // touch the live DOM (insert the wrapper, hide the textarea) once both
    // succeed - so a construction error here never leaves the page with a
    // hidden textarea and no editor to show for it.
    var wrapper = document.createElement("div");
    wrapper.className = "vd-editor";
    wrapper.id = "vd-editor";

    new CM.EditorView({
      parent: wrapper,
      state: CM.EditorState.create({
        doc: textarea.value,
        extensions: [
          jsonWithFhirpath,
          CM.syntaxHighlighting(jsonHighlightStyle),
          CM.syntaxHighlighting(fhirpathHighlightStyle),
          CM.lineNumbers(),
          CM.highlightActiveLine(),
          CM.highlightActiveLineGutter(),
          CM.drawSelection(),
          CM.foldGutter(),
          CM.bracketMatching(),
          CM.closeBrackets(),
          CM.indentOnInput(),
          CM.indentUnit.of("  "),
          CM.history(),
          CM.highlightSelectionMatches(),
          // RF7: local JSON syntax first, then the server structural +
          // FHIRPath lint, ~400ms after the last keystroke.
          CM.linter(vdLinter, { delay: 400 }),
          CM.lintGutter(),
          // The plain textarea it replaces soft-wraps by default; matching
          // that here avoids a new horizontal scrollbar on long FHIRPath
          // expressions the user never had before.
          CM.EditorView.lineWrapping,
          CM.EditorView.contentAttributes.of({ "aria-label": ariaLabel }),
          // No indentWithTab (RF8): Tab must keep moving focus to the next
          // form control, not indent inside the editor.
          CM.keymap.of(
            [].concat(
              CM.closeBracketsKeymap,
              CM.defaultKeymap,
              CM.historyKeymap,
              CM.foldKeymap,
              CM.searchKeymap
            )
          ),
          CM.EditorView.updateListener.of(function (update) {
            if (!update.docChanged) return;
            textarea.value = update.state.doc.toString();
            // Native-input parity for anything else listening on the form.
            textarea.dispatchEvent(new Event("input", { bubbles: true }));
          }),
        ],
      }),
    });

    textarea.parentNode.insertBefore(wrapper, textarea);
    textarea.classList.add("vd-editor__source--mounted");
  }

  try {
    mount();
  } catch (unavailable) {
    // Degrade silently to the plain textarea (NF3) - nothing above this
    // line touches the live DOM until mount() has fully succeeded.
  }
})();
