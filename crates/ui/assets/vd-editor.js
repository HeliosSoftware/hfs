/*
 * ViewDefinition editor mount (#753; generalized for #838; the guided-form
 * pairing extracted onto `editor-pair.js` in #840).
 *
 * Progressive enhancement over the plain `<textarea class="json-editor"
 * name="json">` in `#vd-editor-form` on `/ui/sql/view-definitions`: if the
 * vendored CodeMirror 6 bundle (`/ui/assets/vendor/codemirror.bundle.js`,
 * global `window.HfsCodeMirror`) and the shared mount helper
 * (`code-editor.js`, global `window.HfsCodeEditor`) both loaded and that
 * textarea exists, this mounts a CodeMirror 6 editor on top of it with two
 * language layers - JSON on the outside, FHIRPath (the `lezer-fhirpath`
 * grammar) injected into the string values of the properties that hold
 * FHIRPath expressions - plus this editor's own local + server lint.
 *
 * The wrapper/sync/back-out plumbing (textarea as source of truth, Tab not
 * captured, aria-label, keyboard focus on the scroller, silent degradation
 * without the bundle or on any construction error) belongs to
 * `code-editor.js`; driving the guided-form card beside the editor - the
 * JSON<->form sync, the validity chip, and the row<->editor cross-highlight
 * - belongs to `editor-pair.js` (#840, extracted from this file's own
 * original #843 implementation). This file only owns what is specific to
 * the ViewDefinition editor itself: the JSON+FHIRPath language, its two
 * HighlightStyles (the JSON one built from `code-editor.js`'s shared
 * preset, the FHIRPath one its own), its lint, and handing the mounted
 * view to `HfsEditorPair.mount`.
 *
 * Server lint (#753): the browser only knows syntax; the server knows FHIR.
 * A local `jsonParseLinter()` pass runs first and, if the JSON itself does
 * not parse, is all that shows - the server is not called for text it
 * cannot even read as JSON. Once the JSON parses, this file POSTs it to
 * `/ui/sql/view-definitions/lint` and translates its `{pointer, span}`
 * diagnostics into CM6 `{from, to}` ranges by walking the *browser's own*
 * syntax tree - the server only ever speaks in JSON pointers, since it has
 * no notion of "line 4, column 12" for a document it never parsed into a
 * CM6 tree.
 *
 * Without the bundle, without the helper, without JS, or if anything below
 * throws, this file does nothing (or backs out cleanly) for the editor
 * itself and the page is the textarea as it is today - `editor-pair.js`
 * then drives that plain textarea instead (`getDoc` reads its value,
 * `setDoc` writes it and fires `input`, sync listens for `input` on it
 * directly), no CodeMirror concept involved. Diagnostics, completion, and
 * i18n for the lint are out of scope here.
 */
(function () {
  "use strict";

  var CodeEditor = window.HfsCodeEditor;
  var CM = window.HfsCodeMirror;
  var EditorPair = window.HfsEditorPair;

  var form = document.getElementById("vd-editor-form");
  var textarea = form ? form.querySelector('textarea[name="json"]') : null;
  if (!textarea) return;

  var grid = document.getElementById("vd-editor-grid");
  var view = null;

  if (CodeEditor && CM) {
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

    var propertyKey = function (propertyNameNode, input) {
      var raw = input.read(propertyNameNode.from, propertyNameNode.to);
      return raw.length >= 2 && raw.charAt(0) === '"' && raw.charAt(raw.length - 1) === '"'
        ? raw.slice(1, -1)
        : raw;
    };

    var fhirpathLanguage = CM.LRLanguage.define({ parser: CM.fhirpath.parser });

    var nestFhirpath = function (node, input) {
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
      // before parsing; the FHIRPath grammar just
      // fails to parse cleanly there, which is fine - the string still
      // renders, only without FHIRPath coloring: it degrades to a plain
      // JSON string.
      if (input.read(from, to).indexOf("\\") !== -1) return null;

      return { parser: fhirpathLanguage.parser, overlay: [{ from: from, to: to }] };
    };

    var jsonWithFhirpath = new CM.LanguageSupport(
      CM.jsonLanguage.configure({ wrap: CM.parseMixed(nestFhirpath) })
    );

    /* ---- Highlighting: the shared JSON preset (`code-editor.js`, #840)
     * for the outer grammar, plus this editor's own FHIRPath HighlightStyle
     * for the language injected into it - two HighlightStyles, each scoped
     * to its own language, so the same generic tags (tags.string,
     * tags.number, ...) that both grammars happen to use can still be
     * colored differently depending on which side of the injection they
     * came from. Classes only - every color lives in app.css as a CSS
     * variable, never a fixed value from here. */
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
        // No dedicated FHIRPath punctuation variable (the palette below has 7 --fp-*
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

    /* ---- Server lint (#753) --------------------------------------------
     *
     * CM6's own lint plugin already discards a stale async result on its
     * own (it compares the doc captured when a lint run started against the
     * live doc before dispatching), so nothing here needs its own
     * generation counter for that.
     */

    var unescapePointerSegment = function (segment) {
      return segment.replace(/~1/g, "/").replace(/~0/g, "~");
    };

    var pointerSegments = function (pointer) {
      return pointer === "" ? [] : pointer.slice(1).split("/").map(unescapePointerSegment);
    };

    var propertyKeyText = function (propertyNameNode, doc) {
      var raw = doc.sliceString(propertyNameNode.from, propertyNameNode.to);
      return raw.length >= 2 && raw.charAt(0) === '"' && raw.charAt(raw.length - 1) === '"'
        ? raw.slice(1, -1)
        : raw;
    };

    /* The Property node inside `objectNode` whose PropertyName reads `key`. */
    var findProperty = function (objectNode, key, doc) {
      for (var child = objectNode.firstChild; child; child = child.nextSibling) {
        if (child.name !== "Property") continue;
        var nameNode = child.firstChild;
        if (
          nameNode &&
          nameNode.name === "PropertyName" &&
          propertyKeyText(nameNode, doc) === key
        ) {
          return child;
        }
      }
      return null;
    };

    /* A Property's value is its last child (PropertyName and ":" come first). */
    var propertyValue = function (propertyNode) {
      var last = propertyNode.lastChild;
      return last && last.name !== ":" ? last : null;
    };

    var arrayItemAt = function (arrayNode, index) {
      var i = 0;
      for (var child = arrayNode.firstChild; child; child = child.nextSibling) {
        if (child.name === "[" || child.name === "]" || child.name === ",") continue;
        if (i === index) return child;
        i++;
      }
      return null;
    };

    /* Walks the JSON syntax tree from its root value, following `segments`
     * (an object key by name, an array index numerically), and returns the
     * node at that position - or null if the path does not resolve against
     * this document's actual shape (painted on line 1 by the caller in
     * that case). */
    var resolveBySegments = function (tree, doc, segments) {
      var node = tree.topNode.getChild("Object") || tree.topNode.firstChild;
      if (!node) return null;
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
    };

    var resolveByPointer = function (tree, doc, pointer) {
      return resolveBySegments(tree, doc, pointerSegments(pointer));
    };

    var fallbackRange = function (doc) {
      var line = doc.line(1);
      return { from: line.from, to: line.to };
    };

    /* For an Object/Array (which may span many lines), just the first
     * line - from its opening "{"/"[" to wherever that line ends. A scalar
     * value is already within one line, so its own range is used as-is. */
    var valueRange = function (doc, node) {
      if (node.name === "Object" || node.name === "Array") {
        var line = doc.lineAt(node.from);
        return { from: node.from, to: Math.min(line.to, node.to) };
      }
      return { from: node.from, to: node.to };
    };

    /* UnknownKey locates the *key*, not the value: resolve everything but the
     * pointer's last segment to find the containing object, then find that
     * segment's own PropertyName inside it. */
    var unknownKeyRange = function (tree, doc, pointer) {
      var cut = pointer.lastIndexOf("/");
      var parent = resolveByPointer(tree, doc, pointer.slice(0, cut));
      if (!parent || parent.name !== "Object") return null;
      var property = findProperty(parent, unescapePointerSegment(pointer.slice(cut + 1)), doc);
      var nameNode = property ? property.firstChild : null;
      return nameNode ? { from: nameNode.from, to: nameNode.to } : null;
    };

    /* FhirPathSyntax/UndeclaredConstant: the span is a char offset into the
     * *content* of the pointed-at string (excluding its quotes). Escapes are
     * not unescaped anywhere in this pipeline (server or client), so
     * a span computed against unescaped text would misalign the moment the
     * string contains one - degrade to underlining the whole string instead,
     * exactly like the server's own FhirPathSyntax message already does when
     * it detects an escape (see lint.rs). */
    var spanRange = function (doc, node, span) {
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
    };

    var diagnosticRange = function (view, diagnostic) {
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
    };

    var toCmDiagnostic = function (view, serverDiagnostic) {
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
    };

    var fetchServerDiagnostics = function (view) {
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
          // Network failures, 5xx, 401 - never a visible or console.error
          // failure. Local diagnostics (if any) keep working regardless.
          if (window.console && window.console.debug) {
            window.console.debug("vd-editor: server lint unavailable", error);
          }
          return [];
        });
    };

    var jsonSyntaxLinter = CM.jsonParseLinter();

    var vdLinter = function (view) {
      var syntaxErrors = jsonSyntaxLinter(view);
      return syntaxErrors.length > 0 ? syntaxErrors : fetchServerDiagnostics(view);
    };

    /* ---- Mount ------------------------------------------------------- */

    view = CodeEditor.mount(textarea, {
      language: jsonWithFhirpath,
      highlight: [
        CM.syntaxHighlighting(CodeEditor.jsonHighlight()),
        CM.syntaxHighlighting(fhirpathHighlightStyle),
      ],
      // Local JSON syntax first, then the server structural + FHIRPath
      // lint, ~400ms after the last keystroke. `editor-pair.js` adds its
      // own extensions (the guided-form sync listener, the cross-highlight
      // decoration field) onto this same view once it mounts, below.
      extensions: [CM.linter(vdLinter, { delay: 400 }), CM.lintGutter()],
      fold: true,
      wrapperClass: "vd-editor",
      id: "vd-editor",
    });
  }

  // #843/#840: the guided-form card beside the editor - JSON<->form sync,
  // the validity chip, and the row<->editor cross-highlight - is
  // `editor-pair.js`'s own concern; `view` is `null` here if CodeMirror
  // never mounted, and the pair drives the plain textarea in that case
  // instead. Missing `grid` (a page fragment other than this one's own
  // template) or a missing `editor-pair.js`/`editor-form.js` load leaves
  // the editor above exactly as it is and touches nothing else.
  if (EditorPair) EditorPair.mount({ textarea: textarea, view: view, grid: grid });
})();
