/*
 * ViewDefinition editor mount (#753; generalized for #838; the guided-form
 * host, its two-way sync, and the row<->editor cross-highlight wired on in
 * #843).
 *
 * Progressive enhancement over the plain `<textarea class="json-editor"
 * name="json">` in `#vd-editor-form` on `/ui/sql/view-definitions`: if the
 * the vendored CodeMirror 6 bundle (`/ui/assets/vendor/codemirror.bundle.js`,
 * global `window.HfsCodeMirror`) and the shared mount helper
 * (`code-editor.js`, global `window.HfsCodeEditor`) both loaded and that
 * textarea exists, this mounts a CodeMirror 6 editor on top of it with two
 * language layers - JSON on the outside, FHIRPath (the `lezer-fhirpath`
 * grammar) injected into the string values of the properties that hold
 * FHIRPath expressions - plus this editor's own local + server lint.
 *
 * The wrapper/sync/back-out plumbing (textarea as source of truth, Tab not
 * captured, aria-label, silent degradation without the bundle or on any
 * construction error) belongs to `code-editor.js` and is not duplicated
 * here - this file only owns what is specific to the ViewDefinition editor:
 * the JSON+FHIRPath language, its two HighlightStyles, its lint, and (#843)
 * driving the guided-form card beside it (`#vd-editor-grid`,
 * `window.HfsEditorForm`) as one document with two views:
 *
 *   - a form-driven change lands in the editor as one minimal transaction
 *     (common-prefix/common-suffix diff, `minimalChange` below) tagged with
 *     `FormOriginEffect`, so it is undoable, does not move the scroll or the
 *     caret, and - because `code-editor.js`'s own listener fires on any
 *     `docChanged` update regardless of cause - still updates the hidden
 *     textarea and fires `input` exactly like a manual edit;
 *   - an editor change that is NOT that echo (`FormOriginEffect` absent from
 *     its transactions) is parsed 600ms after the last keystroke and, if it
 *     parses and its canonical form actually changed, re-requests the panel
 *     alone - invalid JSON never reaches the server; the guided form keeps
 *     its rows and a validity chip switches to a short client-side "invalid
 *     JSON" state instead.
 *
 * Hovering or focusing a row paints the lines its node occupies in the
 * editor (a CodeMirror `StateField` of line decorations, `cm-line--hit` in
 * `app.css`, since this page's JSON pane has no server-rendered
 * `.json-line[data-jpath]` the way the Resource Editor's does); moving the
 * cursor in the editor does the reverse, 120ms after it settles, lighting
 * the row for the node it now sits in (or the nearest ancestor with one).
 * Both directions resolve a node through the browser's own CodeMirror
 * syntax tree, walking it by dotted path (`select.0.column.0.path`) or, in
 * reverse, by ancestor - never by re-parsing the text by hand. Reveal stays
 * inside each pane's own scroll container, never the page's.
 *
 * Without the bundle, without the helper, without JS, or if anything below
 * throws, this file does nothing (or backs out cleanly) for the editor
 * itself and the page is the textarea as it is today - the guided-form
 * wiring then drives that plain textarea instead (`getDoc` reads its value,
 * `setDoc` writes it and fires `input`, sync listens for `input` on it
 * directly), no CodeMirror concept involved. Diagnostics, completion, and
 * i18n for the lint are out of scope here.
 *
 * `minimalChange` is exported for its own unit test
 * (`crates/ui/e2e/unit/vd-editor.test.cjs`) via the same UMD-ish shape
 * `assets/combobox.js` uses; `mount` only ever runs when a `document`
 * exists, so requiring this file under Node defines the functions and does
 * nothing else.
 */
(function (root, factory) {
  "use strict";

  var api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root && root.document) api.mount(root);
})(typeof window !== "undefined" ? window : null, function () {
  "use strict";

  /* ---- minimal single-range diff (#843) ----------------------------------
   *
   * The common-prefix/common-suffix diff between two strings, as the one
   * `{from, to, insert}` CodeMirror change that turns `oldText` into
   * `newText` - the smallest edit that does the job, not a general diff: a
   * single contiguous replaced range is exactly what preserves every
   * position outside it (so CodeMirror's own position mapping leaves the
   * caret, the scroll offset, and anything else anchored elsewhere in the
   * document undisturbed) and folds into one undo step. `null` when the two
   * texts are identical - nothing changed, so `setDoc` below dispatches no
   * transaction at all for that case.
   */
  function minimalChange(oldText, newText) {
    if (oldText === newText) return null;

    var oldLen = oldText.length;
    var newLen = newText.length;
    var max = Math.min(oldLen, newLen);

    var prefix = 0;
    while (prefix < max && oldText.charCodeAt(prefix) === newText.charCodeAt(prefix)) {
      prefix++;
    }

    var maxSuffix = max - prefix;
    var suffix = 0;
    while (
      suffix < maxSuffix &&
      oldText.charCodeAt(oldLen - 1 - suffix) === newText.charCodeAt(newLen - 1 - suffix)
    ) {
      suffix++;
    }

    return {
      from: prefix,
      to: oldLen - suffix,
      insert: newText.slice(prefix, newLen - suffix),
    };
  }

  /* ---- completion: pure helpers (#821) ------------------------------------
   *
   * `vdCompletionSource` (below, only defined once CodeMirror is actually
   * mounted - it closes over `CM`) locates *where* the cursor is and asks
   * `POST /ui/sql/view-definitions/complete` *what* fits there; these four
   * functions are the part of "what to do with the answer" that needs no
   * CodeMirror object at all - given as plain values, so they are exported
   * (alongside `minimalChange`) for `vd-editor.test.cjs` to exercise
   * directly under Node.
   */

  /* The JSON skeleton a freshly-inserted key gets, keyed by the `detail`
   * kind `/complete`'s `kind: "key"` response carries for it (the same
   * `helios_sof::lint::KeyKind` names `/lint`'s `unknown-key` check already
   * uses) - one representative empty value per kind, valid JSON on its own,
   * with the cursor landing inside it (see `skeletonCursorOffset`) rather
   * than after it, so typing continues straight into the value. `"other"`,
   * and anything this client does not otherwise recognize, is `null` - not
   * a guess at a real default.
   */
  function skeletonForDetail(detail) {
    switch (detail) {
      case "string":
        return '""';
      case "boolean":
        return "true";
      case "number":
        return "0";
      case "string[]":
        return '[""]';
      case "object":
        return "{}";
      case "object[]":
        return "[{}]";
      default:
        return "null";
    }
  }

  /* Where inside a `skeletonForDetail` result the cursor belongs - inside
   * the empty string/object it just inserted, so the next keystroke fills
   * the value in directly; a skeleton with no natural "inside" (`true`,
   * `0`, `null`) instead places it right after, ready to overtype. */
  function skeletonCursorOffset(skeleton) {
    switch (skeleton) {
      case '""':
        return 1;
      case '[""]':
        return 2;
      case "{}":
        return 1;
      case "[{}]":
        return 2;
      default:
        return skeleton.length;
    }
  }

  /* Classifies one candidate "new key" gap inside a JSON `Object` node and
   * says which comma(s) an insertion there needs to keep the document
   * valid - pure, given only the direct child node names of that `Object`
   * in document order (e.g. `["{", "Property", ",", "Property", "}"]`,
   * `arrayItemAt`'s sibling-walk equivalent for an `Object`) and the index
   * of the child immediately before the gap (`-1` for the gap right after
   * `"{"`, before any child).
   *
   * A gap counts as a key position right after `"{"`, right after `","`, or
   * right after a `"Property"` with no comma of its own yet - the last one
   * reached whenever the user starts a new key before adding the separating
   * comma themselves (or the document is mid-edit and missing one already),
   * which `keyContextAt` below folds into the exact same "gap" handling
   * rather than treating as a separate, unrecognized position. Anything
   * else (right after `"}"`, i.e. past the object entirely) is `null` - not
   * a key position at all. `leadingComma`/`trailingComma` say whether the
   * insertion itself must supply the comma on that side; a `","` already
   * sitting there needs no more help. */
  function classifyObjectGap(childNames, beforeIndex) {
    var before = beforeIndex >= 0 ? childNames[beforeIndex] : null;
    var isKeyPosition = before === null || before === "{" || before === "," || before === "Property";
    if (!isKeyPosition) return null;
    var after = beforeIndex + 1 < childNames.length ? childNames[beforeIndex + 1] : null;
    return {
      leadingComma: before === "Property",
      trailingComma: after === "Property",
    };
  }

  /* `classifyObjectGap` above, fed from a JSON `Object`'s *real* direct
   * children (`children`, each a plain `{name, from, to}` - a live
   * `SyntaxNode` already duck-types this, so `keyContextAt` below passes
   * those straight through with no conversion) instead of a hand-picked
   * name list, plus the one bit of real-tree noise a plain array of names
   * can't express: `lezer-json`'s own error recovery splices a zero-width
   * node (named `"⚠"`, `from === to`) into exactly this gap - at the same
   * offset as whichever real token follows it, a trailing `","` before the
   * object's own `"}"`, or a missing one between two `Property`s - the
   * moment the object does not simply end there. That placeholder is never
   * a real "before" sibling; skipped here before building `classifyObjectGap`'s
   * own `childNames`/`beforeIndex` inputs, since left in, its `.to <= pos`
   * (true at the very `pos` the real `","`/`Property` one slot earlier
   * already satisfies) silently overwrites the right answer with a name
   * `classifyObjectGap` does not recognize - collapsing "right after a
   * comma" (and "between two properties with a missing one") to "not a key
   * position" for any object that is not already at its very end. */
  function objectGapAt(children, pos) {
    var childNames = [];
    var beforeIndex = -1;
    for (var i = 0; i < children.length; i++) {
      var child = children[i];
      if (child.from === child.to) continue;
      childNames.push(child.name);
      if (child.to <= pos) beforeIndex = childNames.length - 1;
    }
    return classifyObjectGap(childNames, beforeIndex);
  }

  /* The literal text (and, within it, the offset the cursor belongs at) for
   * inserting a brand-new `"label": skeleton` property, wrapped in whatever
   * leading/trailing comma text (`","` or `""`) `classifyObjectGap` above
   * decided the surrounding gap needs. */
  function buildKeyInsertion(label, skeleton, leadingComma, trailingComma) {
    var body = '"' + label + '": ' + skeleton;
    var cursor = leadingComma.length + (body.length - skeleton.length) + skeletonCursorOffset(skeleton);
    return { text: leadingComma + body + trailingComma, cursor: cursor };
  }

  /* ---- completion: FHIRPath char-offset conversion (#821) -----------------
   *
   * `/complete`'s `kind: "fhirpath"` request and response both count
   * `cursor`/`from` in Unicode code points ("chars", matching Rust's own
   * `str::chars()`), never CodeMirror's own UTF-16 code-unit document
   * positions - identical for the ASCII/BMP text a FHIRPath expression
   * almost always is, but not in general (an astral character - e.g. an
   * emoji some human-readable `%constant` value happens to contain - is one
   * JS UTF-16 code unit pair but a single Unicode code point). These two
   * pure conversions are the only place that distinction matters. */

  /* The code-point count of `text.slice(0, utf16Offset)` - `utf16Offset`
   * itself when `text` holds no astral character, since every code unit is
   * then also its own code point. */
  function codePointOffset(text, utf16Offset) {
    return Array.from(text.slice(0, utf16Offset)).length;
  }

  /* The inverse: the UTF-16 code-unit offset into `text` that is
   * `codePoints` Unicode code points in - `text.length` if `text` has fewer
   * code points than that. */
  function utf16OffsetForCodePoints(text, codePoints) {
    var offset = 0;
    var seen = 0;
    while (seen < codePoints && offset < text.length) {
      var code = text.codePointAt(offset);
      offset += code > 0xffff ? 2 : 1;
      seen++;
    }
    return offset;
  }

  /* ---- diagnostic actions: structural JSON edits, pure pieces (#821) -----
   *
   * `POST /lint`'s `fixes` are intentions against the document's own
   * structure, addressed by RFC 6901 pointer - never a text position, since
   * the server has no notion of one. Turning one into a CodeMirror change is
   * "resolve the pointer against the *live* syntax tree, then compute a
   * range" - the resolving half needs a real tree (`mount`'s own
   * `resolvePropertyByPointer`/`renameKeyChange`/`removeKeyChange`/
   * `setStringChange`, closing over `CM`, further down this file), but the
   * range/text arithmetic itself does not. These three are that pure half -
   * given already-resolved node positions (or, for the escape, a plain
   * string) they need no CodeMirror object at all, so `vd-editor.test.cjs`
   * exercises them directly under Node with hand-built node stubs.
   */

  /* The `[from, to)` range of a JSON string's *content*, excluding its
   * surrounding quotes - shared by a key rename (on the `Property`'s
   * `PropertyName`) and a `set-string` fix (on the value's own `String`),
   * both of which replace only what sits inside the quotes. `Math.max`
   * guards the degenerate `""` case, where `to` would otherwise land one
   * before `from`. */
  function stringContentRange(node) {
    return { from: node.from + 1, to: Math.max(node.from + 1, node.to - 1) };
  }

  /* `value`, ready to sit inside a JSON string's quotes: `\` first (so a
   * `"` this produces is not itself re-escaped a second time), then `"`. */
  function escapeJsonStringContent(value) {
    return value.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
  }

  /* The `[from, to)` range that removes `propertyNode` (a JSON `Property`)
   * *and* exactly the comma that would otherwise dangle - the next one if
   * it has one, else the previous one - so the result is always valid JSON
   * with every other property's own indentation untouched:
   *
   *   - a following comma exists: delete from wherever the *previous* token
   *     ends (the prior comma, or the object's own "{" when this is the
   *     first property) through that following comma - the removed
   *     property's own leading whitespace/newline goes with it, so the next
   *     property's is the only one left between the two survivors.
   *   - no following comma, but a preceding one does (this was the last
   *     property): delete from that comma itself (not just after it - the
   *     comma must go too, since the property before it is now last)
   *     through the end of this property's own value.
   *   - neither (the object's only property): delete from the "{" through
   *     the end of the value - the same shape as the first case with no
   *     preceding token to speak of.
   *
   * Takes only `propertyNode` - its own `.parent`/`.prevSibling`/
   * `.nextSibling`/`.from`/`.to` are all this needs, so no `doc` parameter:
   * every quantity here is a position, never text. */
  function removeKeyRange(propertyNode) {
    var prev = propertyNode.prevSibling;
    var next = propertyNode.nextSibling;
    var prevComma = prev && prev.name === "," ? prev : null;
    var nextComma = next && next.name === "," ? next : null;
    var openBrace = propertyNode.parent.firstChild;
    if (nextComma) {
      return { from: prevComma ? prevComma.to : openBrace.to, to: nextComma.to };
    }
    if (prevComma) {
      return { from: prevComma.from, to: propertyNode.to };
    }
    return { from: openBrace.to, to: propertyNode.to };
  }

  /* ---- mount -------------------------------------------------------------- */

  function mount(root) {
    var document = root.document;
    var CodeEditor = root.HfsCodeEditor;
    var CM = root.HfsCodeMirror;
    var EditorForm = root.HfsEditorForm;

    var form = document.getElementById("vd-editor-form");
    var textarea = form ? form.querySelector('textarea[name="json"]') : null;
    if (!textarea) return;

    var grid = document.getElementById("vd-editor-grid");
    // #821: the translated "required" marker completion items for a
    // structural key carry - read once here (rather than inside the
    // completion source itself) since `grid` is already looked up at this
    // point in `mount`; `undefined` when absent (no `data-msg-required`, or
    // no grid at all) degrades to no marker rather than an English literal.
    var requiredLabel = grid ? grid.dataset.msgRequired : undefined;

    // The CodeMirror mount's own `updateListener` extension needs this
    // reference before the guided-form wiring below exists to fill it in
    // (`formApi`, `lastSynced`, ...) - safe, because `handleCmUpdate` is a
    // hoisted declaration and is never actually called until a real edit,
    // long after every `var` it closes over has been assigned by the rest
    // of this function running top to bottom.
    var formApi = { refresh: function () {} };
    var lastSynced = null;
    var syncTimer = null;
    var invalidChip = null;
    var FormOriginEffect = null;
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

      /* ---- Highlighting: two HighlightStyles, each scoped to its own
       * language, so the same generic tags (tags.string, tags.number, ...)
       * that both the JSON grammar and lezer-fhirpath happen to use can still
       * be colored differently depending on which side of the injection they
       * came from. Classes only - every color lives in app.css as a CSS
       * variable, never a fixed value from here. */
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

      /* The inverse of `arrayItemAt` (#843, cross-highlight): the index of
       * `target` among `arrayNode`'s own items, or -1 if `target` is not one
       * of them. Compared by range rather than reference - a node handed
       * back by `tree.resolveInner` is not guaranteed to be the exact object
       * identity `firstChild`/`nextSibling` walk past, only the same
       * `{from, to}` span. */
      var arrayIndexOfChild = function (arrayNode, target) {
        var i = 0;
        for (var child = arrayNode.firstChild; child; child = child.nextSibling) {
          if (child.name === "[" || child.name === "]" || child.name === ",") continue;
          if (child.from === target.from && child.to === target.to) return i;
          i++;
        }
        return -1;
      };

      /* Walks the JSON syntax tree from its root value, following `segments`
       * (an object key by name, an array index numerically), and returns the
       * node at that position - or null if the path does not resolve against
       * this document's actual shape (painted on line 1 by the caller in
       * that case). The one walk both path notations below share: an RFC
       * 6901 pointer and the editor rows' dotted form differ only in how
       * they cut a path into segments, never in how a segment locates a
       * child. */
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

      /* `select.0.column.0.path` - the same dotted form the guided-form
       * rows are keyed on (`editor::path_to_string`, mirrored client-side by
       * nothing more than a "." split: FHIR element names never contain a
       * dot themselves, so no escaping is needed here the way `~0`/`~1` is
       * for a pointer segment that does). */
      var dottedSegments = function (path) {
        return path === "" ? [] : path.split(".");
      };

      var resolveByDottedPath = function (tree, doc, path) {
        return resolveBySegments(tree, doc, dottedSegments(path));
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
            if (root.console && root.console.debug) {
              root.console.debug("vd-editor: server lint unavailable", error);
            }
            return [];
          });
      };

      var jsonSyntaxLinter = CM.jsonParseLinter();

      /* #821: the diagnostics the most recently *completed* lint pass
       * produced - local JSON syntax errors, or the server's structural +
       * FHIRPath checks, whichever branch of `vdLinter` below actually ran.
       * Read by the save-confirmation submit handler further down `mount`:
       * a Save pressed while a server round trip is still in flight sees
       * whatever this held before that pass started, not a half-finished
       * one - there is no reliable "pass in progress" signal to block on
       * instead, and the previous pass's result is the closest
       * approximation available without delaying the click on the network. */
      var lastLintDiagnostics = [];

      var recordLintResult = function (diagnostics) {
        lastLintDiagnostics = diagnostics;
        return diagnostics;
      };

      var vdLinter = function (view) {
        var syntaxErrors = jsonSyntaxLinter(view);
        if (syntaxErrors.length > 0) return recordLintResult(syntaxErrors);
        return fetchServerDiagnostics(view).then(recordLintResult);
      };

      /* ---- Ctrl+. : apply the fix under the cursor (#821) -----------------
       *
       * `Mod-.` collects every action of every diagnostic whose range
       * touches the current cursor or selection via `forEachDiagnostic` -
       * the live set CM6's own lint state already tracks, so no separate
       * bookkeeping of "diagnostics at the cursor" is needed here. A
       * zero-width selection (the common case, a plain cursor) "touches" a
       * diagnostic range it sits anywhere inside, endpoints included; a real
       * selection "touches" one it actually overlaps. Exactly one action
       * across all of them applies it directly; more than one opens the
       * lint panel (`openLintPanel` - navigable and clickable from there,
       * `lintKeymap` below adds F8/Ctrl-Shift-M to reach it by keyboard
       * too); none returns `false`, letting `.` fall through to its normal
       * self-insertion.
       */
      var diagnosticTouchesSelection = function (diagFrom, diagTo, selFrom, selTo) {
        if (selFrom === selTo) return selFrom >= diagFrom && selFrom <= diagTo;
        return selFrom < diagTo && selTo > diagFrom;
      };

      var applyFixAtCursor = function (view) {
        var sel = view.state.selection.main;
        var actions = [];
        CM.forEachDiagnostic(view.state, function (diagnostic, from, to) {
          if (!diagnosticTouchesSelection(from, to, sel.from, sel.to)) return;
          (diagnostic.actions || []).forEach(function (action) {
            actions.push(action);
          });
        });
        if (actions.length === 1) {
          actions[0].apply(view, sel.from, sel.to);
          return true;
        }
        if (actions.length > 1) return CM.openLintPanel(view);
        return false;
      };

      /* ---- Completion (#821) ----------------------------------------------
       *
       * The browser resolves *where* the cursor is (this section); the
       * server (`POST /ui/sql/view-definitions/complete`, `vd_complete.rs`)
       * decides *what* fits there. `vdCompletionSource`, registered as
       * `code-editor.js`'s `completion` option below, is the single entry
       * point CodeMirror calls on every keystroke (`activateOnTyping`) and
       * on Ctrl-Space (`completionKeymap`): it classifies `context.pos`
       * against the browser's own syntax tree into exactly one of two
       * request shapes - a structural JSON key (`keyContextAt`) or a
       * partial FHIRPath expression (`fhirpathContextAt`, gated by the same
       * injection rule `nestFhirpath` above uses, so completion and syntax
       * coloring never disagree about which strings hold FHIRPath) - or
       * `null` for anywhere else, which shows no popup at all.
       *
       * Every fetch is same-origin, `AbortController`-linked to CodeMirror's
       * own `context.addEventListener("abort", ...)` (fired the moment a
       * newer keystroke supersedes this request), and any failure - network,
       * a non-2xx status, a body that is not the JSON this expects -
       * degrades to `null` with a `console.debug`, exactly like the lint
       * fetch above: a completion source is not a place to surface an error
       * the user did not ask for.
       */

      /* The `Object` node that is `pos`'s innermost ancestor - `pos` itself
       * when `resolveInner` already lands there (an empty object, or a
       * whitespace gap between children with no leaf node of its own to
       * resolve to), or its parent when `pos` resolved to one of that
       * `Object`'s own direct children (`"{"`, `","`, `"}"`, or a
       * `"Property"`) instead. `null` when neither is an `Object` at all. */
      var objectAncestor = function (node) {
        if (node.name === "Object") return node;
        return node.parent && node.parent.name === "Object" ? node.parent : null;
      };

      /* `resolveInner(pos, -1)` picks the *deepest* node ending exactly at
       * `pos`, not the outermost one - right after a property's value
       * (`{"a": 1|}`), that is the value node itself (`Number`, `String`,
       * a nested `Object`/`Array`, ...), not the `Property` wrapping it,
       * even though both end at the same offset. Climbs from `node` through
       * every ancestor whose own end also lands exactly on `pos` - a value
       * up to its `Property`, and a `Property` up to its `Object` only if
       * the object itself has no closing `"}"` yet - so the gap scan below
       * always sees a `Property` (or the enclosing `Object`) as `pos`'s
       * immediate predecessor, never a value node one level too deep. Never
       * climbs out of an open key string (`node.name === "PropertyName"`
       * is checked, and returns, before this ever runs). */
      var climbToBoundary = function (node, pos) {
        while (node.parent && node.to === pos && node.parent.to === pos) {
          node = node.parent;
        }
        return node;
      };

      /* Where `pos` sits relative to a JSON `Object`'s structure - either
       * inside an already-open key string (renaming it, or resuming a key
       * that has no `:`/value yet), or in a "new key" gap between its
       * children - or `null` for anywhere else (a value position, an array,
       * or past the object's own closing `"}"`, none of which this editor
       * offers key completion for).
       *
       * Returns, for an open string: `{openString: true, from, contentFrom,
       * contentTo, propertyEnd, hasColon, objectNode, excludeProperty}`.
       * For a gap: `{openString: false, from, objectNode, excludeProperty:
       * null, leadingComma, trailingComma}` (`classifyObjectGap` above
       * supplies the last two). `objectNode`/`excludeProperty` are read by
       * the completion *source* (building the `pointer`/`present` request
       * fields); `contentFrom`/`contentTo`/`propertyEnd`/`hasColon`/
       * `leadingComma`/`trailingComma` are read by `applyKeyCompletion`,
       * which calls this again fresh at accept time rather than trusting
       * whatever this returned when the request went out - the document may
       * have changed underneath it since. */
      var keyContextAt = function (tree, doc, pos) {
        var node = tree.resolveInner(pos, -1);

        if (node.name === "PropertyName") {
          var property = node.parent;
          var objectNode = property && property.parent && property.parent.name === "Object" ? property.parent : null;
          if (!objectNode) return null;
          var contentFrom = node.from + 1;
          var contentTo = Math.max(contentFrom, node.to - 1);
          if (pos < contentFrom || pos > contentTo) return null;
          var colon = node.nextSibling;
          return {
            openString: true,
            from: contentFrom,
            contentFrom: contentFrom,
            contentTo: contentTo,
            propertyEnd: property.to,
            hasColon: !!colon && colon.name === ":",
            objectNode: objectNode,
            excludeProperty: property,
          };
        }

        var ancestor = objectAncestor(climbToBoundary(node, pos));
        if (!ancestor) return null;

        var children = [];
        for (var child = ancestor.firstChild; child; child = child.nextSibling) children.push(child);
        var gap = objectGapAt(children, pos);
        if (!gap) return null;

        return {
          openString: false,
          from: pos,
          objectNode: ancestor,
          excludeProperty: null,
          leadingComma: gap.leadingComma,
          trailingComma: gap.trailingComma,
        };
      };

      /* The keys `objectNode`'s own `Property` children already declare, in
       * document order - `excludeProperty` (compared by range, not
       * reference: see `arrayIndexOfChild`'s own comment on why a node
       * handed back by `resolveInner` cannot be compared by identity
       * against one reached by a sibling walk) left out, so completing the
       * very key currently being typed or renamed never excludes itself. */
      var presentKeys = function (objectNode, doc, excludeProperty) {
        var keys = [];
        for (var child = objectNode.firstChild; child; child = child.nextSibling) {
          if (child.name !== "Property") continue;
          if (
            excludeProperty &&
            child.from === excludeProperty.from &&
            child.to === excludeProperty.to
          ) {
            continue;
          }
          var nameNode = child.firstChild;
          if (nameNode && nameNode.name === "PropertyName") {
            keys.push(propertyKeyText(nameNode, doc));
          }
        }
        return keys;
      };

      var escapePointerSegment = function (segment) {
        return segment.replace(/~/g, "~0").replace(/\//g, "~1");
      };

      /* The RFC 6901 pointer for `node` itself (not a descendant) - the
       * inverse of `resolveByPointer` above, and the same ancestor walk
       * `nodePathAtCursor` already does for the dotted row-path form, with
       * pointer escaping instead of a plain `.` join. Shared by both
       * `completeKey` (the containing `Object`'s own pointer) and
       * `completeFhirpath` (the `String` node's own pointer). */
      var pointerForNode = function (node, doc) {
        var segments = [];
        while (node) {
          if (node.name === "Property") {
            var nameNode = node.firstChild;
            if (nameNode && nameNode.name === "PropertyName") {
              segments.unshift(escapePointerSegment(propertyKeyText(nameNode, doc)));
            }
            node = node.parent;
            continue;
          }
          var parent = node.parent;
          if (!parent) break;
          if (parent.name === "Array") {
            var arrayIndex = arrayIndexOfChild(parent, node);
            if (arrayIndex !== -1) segments.unshift(String(arrayIndex));
          }
          node = parent;
        }
        return segments.length ? "/" + segments.join("/") : "";
      };

      /* FHIRPath injection context at `pos` - the same "is this string a
       * FHIRPath expression" test `nestFhirpath` above applies while
       * building the syntax tree, run again here (not shared - the
       * injection rule itself is left untouched, above) against the
       * finished tree instead of mid-parse, plus the additional constraint
       * that `pos` sits strictly within the string's *content* (excluding
       * its quotes) and that content has no `\` - exactly `nestFhirpath`'s
       * own escape bail-out, so a string this editor declines to inject
       * FHIRPath coloring into is never offered FHIRPath completion either.
       *
       * `pos` almost never resolves to the outer JSON `String` node itself
       * once it has real FHIRPath content: `resolveInner` picks the
       * *deepest* match, which inside injected content is one of that
       * grammar's own nodes (`Identifier`, `Invocation`, a FHIRPath string
       * literal's own `'...'`, itself *also* named `"String"` by
       * `lezer-fhirpath` - confirmed directly against `CM.fhirpath.parser`,
       * not assumed) - so the climb below cannot just stop at the first
       * `"String"` it meets. It keeps going past one whose parent is not
       * `"Property"` (or `"Array"` under one, the `repeat` case) - only the
       * *outer* JSON string has that shape, since a FHIRPath literal's own
       * parent is always that grammar's `"Literal"` wrapper, never a JSON
       * `"Property"`. */
      var fhirpathContextAt = function (tree, doc, pos) {
        var node = tree.resolveInner(pos, -1);
        var property = null;
        var mustBeRepeat = false;
        while (node) {
          if (node.name === "String") {
            var parent = node.parent;
            if (parent && parent.name === "Property") {
              property = parent;
              break;
            }
            if (parent && parent.name === "Array" && parent.parent && parent.parent.name === "Property") {
              property = parent.parent;
              mustBeRepeat = true;
              break;
            }
          }
          node = node.parent;
        }
        if (!property) return null;

        var nameNode = property.firstChild;
        if (!nameNode || nameNode.name !== "PropertyName") return null;
        var key = propertyKeyText(nameNode, doc);
        var matches = mustBeRepeat ? key === "repeat" : EXPRESSION_PROPERTIES.hasOwnProperty(key);
        if (!matches) return null;

        var contentFrom = node.from + 1;
        var contentTo = node.to - 1;
        if (contentTo < contentFrom || pos < contentFrom || pos > contentTo) return null;
        if (doc.sliceString(contentFrom, contentTo).indexOf("\\") !== -1) return null;

        return { stringNode: node, contentFrom: contentFrom, contentTo: contentTo };
      };

      /* Same-origin POST to `/complete`, `AbortController`-linked to
       * CodeMirror's own completion `context` so an in-flight request is
       * cancelled the moment it is superseded - resolves to the parsed
       * response body, or `null` for any failure at all (never a thrown
       * rejection: `vdCompletionSource`'s two callers both just treat a
       * `null` result as "no popup"). */
      var postComplete = function (context, body) {
        var controller = new AbortController();
        context.addEventListener("abort", function () {
          controller.abort();
        });
        return fetch("/ui/sql/view-definitions/complete", {
          method: "POST",
          credentials: "same-origin",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(body),
          signal: controller.signal,
        })
          .then(function (response) {
            if (!response.ok) throw new Error("complete endpoint status " + response.status);
            return response.json();
          })
          .catch(function (error) {
            if (root.console && root.console.debug) {
              root.console.debug("vd-editor: completion unavailable", error);
            }
            return null;
          });
      };

      /* The required-item marker: appended to the kind label the server
       * already sent (`"string"`, `"object[]"`, ...) rather than replacing
       * it, and left off entirely when `requiredLabel` is not available
       * (`mount`'s own read of `data-msg-required` came back empty) - never
       * an English fallback string. */
      var formatKeyDetail = function (item) {
        if (!item.required || !requiredLabel) return item.detail;
        return item.detail ? item.detail + " · " + requiredLabel : requiredLabel;
      };

      /* Re-resolves the key context fresh at `to` (rather than trusting
       * whatever the completion source captured when the request went out -
       * the document, and so the syntax tree, may have changed since) and
       * dispatches the one transaction that inserts/renames a key for
       * `completion` (built by `buildKeyOption` below, `vdSkeleton` its own
       * private field carrying the raw kind - `completion.detail` is by now
       * the *formatted*, translated-marker-appended text, not something
       * `skeletonForDetail` can read back).
       *
       * Renaming/completing an open key string replaces just its content -
       * whatever the user has typed so far - and, only when it has no `:`
       * yet, appends `": " + skeleton` right after the property - both
       * computed from the same fresh snapshot, so the two changes in one
       * `dispatch` never straddle two different documents. A new-key gap
       * replaces `[from, to)` (CodeMirror's own live tracking of how much
       * has been typed since the popup opened) with `"label": skeleton`, comma-
       * wrapped by `buildKeyInsertion`. If the document's shape no longer
       * resolves a key context here at all (a rare edit-while-a-completion-
       * is-open race), this falls back to replacing `[from, to)` with the
       * bare label - never nothing, and never a throw. */
      var applyKeyCompletion = function (view, completion, from, to) {
        var skeleton = completion.vdSkeleton;
        var label = completion.label;
        var fresh = keyContextAt(CM.syntaxTree(view.state), view.state.doc, to);

        if (fresh && fresh.openString) {
          var nameChange = { from: fresh.contentFrom, to: fresh.contentTo, insert: label };
          if (fresh.hasColon) {
            view.dispatch({ changes: nameChange, selection: { anchor: fresh.contentFrom + label.length } });
            return;
          }
          var delta = label.length - (fresh.contentTo - fresh.contentFrom);
          view.dispatch({
            changes: [nameChange, { from: fresh.propertyEnd, insert: ": " + skeleton }],
            selection: { anchor: fresh.propertyEnd + delta + 2 + skeletonCursorOffset(skeleton) },
          });
          return;
        }

        if (fresh && !fresh.openString) {
          var built = buildKeyInsertion(
            label,
            skeleton,
            fresh.leadingComma ? "," : "",
            fresh.trailingComma ? "," : ""
          );
          view.dispatch({
            changes: { from: from, to: to, insert: built.text },
            selection: { anchor: from + built.cursor },
          });
          return;
        }

        view.dispatch({ changes: { from: from, to: to, insert: label } });
      };

      var buildKeyOption = function (item) {
        return {
          label: item.label,
          type: "property",
          detail: formatKeyDetail(item),
          // CodeMirror sorts by `boost` before falling back to alphabetical
          // order - a required key is the one most worth seeing first.
          boost: item.required ? 1 : 0,
          apply: applyKeyCompletion,
          vdSkeleton: skeletonForDetail(item.detail),
        };
      };

      /* `kind: "key"`: `keyCtx.objectNode`'s own pointer and its
       * current keys are the whole request - the popup's `from` is
       * `keyCtx.from` (query time; CodeMirror tracks `to` live against
       * `validFor` as the user keeps typing), matched against typed text by
       * `/^"?[\w]*$/` (an optional leading quote, then word characters -
       * covers both an open key string and typing straight into a gap). */
      var completeKey = function (context, keyCtx, doc) {
        var pointer = pointerForNode(keyCtx.objectNode, doc);
        var present = presentKeys(keyCtx.objectNode, doc, keyCtx.excludeProperty);
        return postComplete(context, { kind: "key", pointer: pointer, present: present }).then(function (data) {
          if (!data || !Array.isArray(data.items) || data.items.length === 0) return null;
          return {
            from: keyCtx.from,
            options: data.items.map(buildKeyOption),
            validFor: /^"?[\w]*$/,
          };
        });
      };

      /* A `function` item inserts `name()` with the cursor between the
       * parens as a plain two-change-free single insert - unlike the
       * key case, nothing here depends on the live syntax tree, so no
       * re-resolution at accept time is needed. `completion.detail` is the
       * catalog's own call signature (`"where(criteria)"`, `"first()"`);
       * a signature with empty parens means the function takes no
       * arguments, so the cursor lands after the whole call instead of
       * inside it. */
      var applyFunctionCompletion = function (view, completion, from, to) {
        var noArgs = /^[A-Za-z_]\w*\(\)$/.test(completion.detail || "");
        var insertText = completion.label + "()";
        var cursor = from + (noArgs ? insertText.length : completion.label.length + 1);
        view.dispatch({ changes: { from: from, to: to, insert: insertText }, selection: { anchor: cursor } });
      };

      /* `kind: "element"`/`"function"`/`"constant"`/`"variable"`:
       * `element`/`constant`/`variable` all take CodeMirror's own default
       * `apply` (a plain `[from, to)` replace with `completion.label`,
       * exactly right since `constant`/`variable` labels already carry
       * their own `%`) - only `function` supplies one.
       *
       * `boost` orders the merged list by kind rather than leaving it to
       * alphabetical chance: a member position can easily return a type's
       * own elements alongside the *entire* function catalog (100+ names),
       * and CodeMirror sorts by `boost` before falling back to alphabetical
       * order - without one, a function whose name sorts late (`where`)
       * competes on equal footing with every element and can end up well
       * past what fits on screen. Elements (what the user is most likely
       * typing a member chain toward) rank above constants/variables, which
       * rank above functions - each kind's own items still sort
       * alphabetically among themselves. */
      var FHIRPATH_KIND_BOOST = { element: 2, constant: 1, variable: 1, function: 0 };

      var buildFhirpathOption = function (item) {
        var boost = FHIRPATH_KIND_BOOST[item.kind] || 0;
        if (item.kind === "function") {
          return {
            label: item.label,
            type: "function",
            detail: item.detail,
            boost: boost,
            apply: applyFunctionCompletion,
          };
        }
        var option = {
          label: item.label,
          type: item.kind === "element" ? "property" : "variable",
          detail: item.detail,
          boost: boost,
        };
        if (item.kind === "element" && item.doc) option.info = item.doc;
        return option;
      };

      /* `kind: "fhirpath"`: a document that does not parse as
       * JSON never reaches the server at all - the field being edited is
       * necessarily inside that same document, so a syntax error anywhere
       * in it means there is no reliable `document.resource`/`%context`
       * type to complete against. `cursor`/`from` cross the code-point/
       * UTF-16 boundary via `codePointOffset`/`utf16OffsetForCodePoints`
       * above; everything else is exactly the fields `vd_complete.rs`
       * documents. */
      var completeFhirpath = function (context, fpCtx, doc) {
        var text = doc.sliceString(fpCtx.contentFrom, fpCtx.contentTo);
        var cursor = codePointOffset(text, context.pos - fpCtx.contentFrom);
        var document;
        try {
          document = JSON.parse(doc.toString());
        } catch (invalidJson) {
          return null;
        }
        var pointer = pointerForNode(fpCtx.stringNode, doc);
        return postComplete(context, {
          kind: "fhirpath",
          pointer: pointer,
          document: document,
          expression: text,
          cursor: cursor,
        }).then(function (data) {
          if (!data || !Array.isArray(data.items) || data.items.length === 0) return null;
          return {
            from: fpCtx.contentFrom + utf16OffsetForCodePoints(text, data.from),
            options: data.items.map(buildFhirpathOption),
            validFor: /^%?[\w]*$/,
          };
        });
      };

      /* The one `CompletionSource` `code-editor.js`'s `completion` option
       * gets: classify, then delegate. Neither classifier touches the
       * network - only the branch that actually matches does, so moving
       * the cursor somewhere this editor has no opinion about never
       * fires a request at all. */
      var vdCompletionSource = function (context) {
        var tree = CM.syntaxTree(context.state);
        var doc = context.state.doc;

        var keyCtx = keyContextAt(tree, doc, context.pos);
        if (keyCtx) return completeKey(context, keyCtx, doc);

        var fpCtx = fhirpathContextAt(tree, doc, context.pos);
        if (fpCtx) return completeFhirpath(context, fpCtx, doc);

        return null;
      };

      /* ---- #843: the transaction-origin marker --------------------------
       *
       * A `StateEffect` carried on the one transaction `setDoc` (below)
       * dispatches for a form-driven change - not a `StateField`, since
       * nothing needs to persist this past the transaction that carries it.
       * `handleCmUpdate`'s sync listener checks for it to skip its own form's
       * echo, without needing to compare document text at all for that case.
       */
      FormOriginEffect = CM.StateEffect.define();

      /* ---- #843: row <-> editor cross-highlight ---------------------------
       *
       * `.editor-row--hit` already exists (the Resource Editor's own
       * `editor-sync.js` set it long before this file did); the JSON side
       * has no equivalent to reach for, because that file's `.json-line
       * [data-jpath]` lines are server-rendered markup this page's JSON pane
       * never has - it is a CodeMirror `EditorView`, not HTML text. A line
       * decoration is CM6's own notion of "paint this line": a `StateField`
       * holding the current decoration set, replaced wholesale on every
       * `HighlightEffect` and reset to none the moment the document itself
       * changes - a line range computed against the old text is meaningless
       * against the new one, and recomputing it is exactly what the next
       * hover or cursor move already does on its own.
       *
       * `hitKey` below is the single source of truth for which one of the
       * two directions (if either) is currently painted, the same
       * mutually-exclusive "one hit at a time" model `editor-sync.js` uses
       * for its own `__hitKey` - hovering a row clears a prior cursor-driven
       * row mark before painting the editor, and vice versa.
       */
      var HighlightEffect = CM.StateEffect.define();
      var lineHighlightMark = CM.Decoration.line({ class: "cm-line--hit" });
      var highlightField = CM.StateField.define({
        create: function () {
          return CM.Decoration.none;
        },
        update: function (decorations, tr) {
          if (tr.docChanged) return CM.Decoration.none;
          for (var i = 0; i < tr.effects.length; i++) {
            if (tr.effects[i].is(HighlightEffect)) return tr.effects[i].value;
          }
          return decorations;
        },
        provide: function (field) {
          return CM.EditorView.decorations.from(field);
        },
      });

      var hitKey = null;
      var selectionTimer = null;

      /* Row -> editor: the line range a row's node occupies, from the line of
       * its own key (its enclosing Property, not just its value - a
       * multi-line object's key sits on the line above its own `{`) through
       * the line its value closes on. An array item has no key of its own,
       * so its node's own `from` (the item's opening bracket/quote/digit)
       * is where the range starts instead. `null` when `path` does not
       * resolve against the document's current shape (a half-typed edit,
       * most often), so the caller paints nothing rather than a stale or
       * wrong range. */
      var nodeLineRange = function (path) {
        var tree = CM.syntaxTree(view.state);
        var node = resolveByDottedPath(tree, view.state.doc, path);
        if (!node) return null;
        var property = node.parent && node.parent.name === "Property" ? node.parent : null;
        return { from: property ? property.from : node.from, to: node.to };
      };

      /* Editor -> row: the dotted path of the node the cursor sits in,
       * climbing the syntax tree from the innermost node at the cursor
       * (`side: -1` so a cursor right after a closing quote still resolves
       * to the string it just left, not whatever follows). Landing inside a
       * Property's own key counts as that property, by construction: a leaf
       * inside `PropertyName`, or `resolveInner` handing back the `Property`
       * node itself outright (a cursor sitting in the gap between key and
       * value resolves no deeper than that), both hit the `node.name ===
       * "Property"` branch and capture that property's key before climbing
       * past it. */
      var nodePathAtCursor = function () {
        var tree = CM.syntaxTree(view.state);
        var doc = view.state.doc;
        var node = tree.resolveInner(view.state.selection.main.head, -1);
        var segments = [];
        while (node) {
          if (node.name === "Property") {
            var nameNode = node.firstChild;
            if (nameNode && nameNode.name === "PropertyName") {
              segments.unshift(propertyKeyText(nameNode, doc));
            }
            node = node.parent;
            continue;
          }
          var parent = node.parent;
          if (!parent) break;
          if (parent.name === "Array") {
            var index = arrayIndexOfChild(parent, node);
            if (index !== -1) segments.unshift(String(index));
          }
          node = parent;
        }
        return segments.join(".");
      };

      /* The row for `path`, or - failing that - its nearest ancestor row
       * (stripping one dotted segment at a time) down to and including the
       * root row itself (`path: ""`, always present: `build_form_pane`
       * gives the document's own top-level object a row). `null` only were
       * the tree walk above to hand back something that shares not even
       * that much with the rendered rows - not reached in practice, since
       * every node `nodePathAtCursor` can name descends from that same
       * root. */
      var rowForPathOrAncestor = function (path) {
        for (;;) {
          var rows = grid.querySelectorAll(".editor-row[data-path]");
          for (var i = 0; i < rows.length; i++) {
            if (rows[i].dataset.path === path) return rows[i];
          }
          if (path === "") return null;
          var cut = path.lastIndexOf(".");
          path = cut === -1 ? "" : path.slice(0, cut);
        }
      };

      /* Container-only reveal (`.editor-tree`'s own scroll, never the
       * page's) - the same shape as `editor-sync.js`'s own `reveal`, kept as
       * its own small copy here rather than imported: that file stays
       * untouched, and importing one six-line helper across a page boundary
       * is not worth the coupling. */
      var revealRow = function (row) {
        var tree = grid.querySelector(".editor-tree");
        if (!tree || !row || tree.scrollHeight <= tree.clientHeight) return;
        var delta = row.getBoundingClientRect().top - tree.getBoundingClientRect().top;
        var top = tree.scrollTop + delta;
        if (top < tree.scrollTop || top > tree.scrollTop + tree.clientHeight - 32) {
          tree.scrollTop = Math.max(0, top - tree.clientHeight / 3);
        }
      };

      /* Clears whichever of the two directions is currently painted - a
       * no-op, no dispatch included, once nothing is. */
      var clearHighlight = function () {
        if (hitKey === null) return;
        hitKey = null;
        clearTimeout(selectionTimer);
        view.dispatch({ effects: HighlightEffect.of(CM.Decoration.none) });
        grid.querySelectorAll(".editor-row--hit").forEach(function (row) {
          row.classList.remove("editor-row--hit");
        });
      };

      /* Row -> editor: paints every line `path`'s node spans and brings the
       * first one into the editor's own viewport - never the page's, since
       * `EditorView.scrollIntoView` only ever moves `.cm-scroller`. */
      var highlightPathInEditor = function (path) {
        var range = nodeLineRange(path);
        if (!range) return;
        var doc = view.state.doc;
        var firstLine = doc.lineAt(range.from).number;
        var lastLine = doc.lineAt(range.to).number;
        var lines = [];
        for (var n = firstLine; n <= lastLine; n++) {
          lines.push(lineHighlightMark.range(doc.line(n).from));
        }
        hitKey = "r:" + path;
        view.dispatch({
          effects: [
            HighlightEffect.of(CM.Decoration.set(lines)),
            CM.EditorView.scrollIntoView(doc.line(firstLine).from, { y: "nearest" }),
          ],
        });
      };

      var handleRowEnter = function (event) {
        var row = event.target.closest ? event.target.closest(".editor-row[data-path]") : null;
        if (!row || !grid.contains(row)) return;
        var key = "r:" + row.dataset.path;
        if (hitKey === key) return;
        clearHighlight();
        highlightPathInEditor(row.dataset.path);
      };

      var handleRowLeave = function (event) {
        var row = event.target.closest ? event.target.closest(".editor-row[data-path]") : null;
        if (!row) return;
        var to = event.relatedTarget;
        if (to && grid.contains(to) && to.closest && to.closest(".editor-row[data-path]")) return;
        clearHighlight();
      };

      /* Editor -> row: 120ms after the cursor last moved (selection change
       * or a keystroke alike - both reach here through the same
       * `updateListener`, registered below), so a fast typist or an
       * in-progress selection drag does not churn the form pane's DOM on
       * every intermediate position. */
      var handleSelectionUpdate = function () {
        clearTimeout(selectionTimer);
        selectionTimer = setTimeout(function () {
          var path;
          try {
            path = nodePathAtCursor();
          } catch (unresolved) {
            path = null;
          }
          var row = path === null ? null : rowForPathOrAncestor(path);
          if (!row) {
            clearHighlight();
            return;
          }
          var key = "c:" + row.dataset.path;
          if (hitKey === key) return;
          clearHighlight();
          hitKey = key;
          row.classList.add("editor-row--hit");
          revealRow(row);
        }, 120);
      };

      /* ---- Mount ------------------------------------------------------- */

      view = CodeEditor.mount(textarea, {
        language: jsonWithFhirpath,
        highlight: [
          CM.syntaxHighlighting(jsonHighlightStyle),
          CM.syntaxHighlighting(fhirpathHighlightStyle),
        ],
        // Local JSON syntax first, then the server structural + FHIRPath
        // lint, ~400ms after the last keystroke; the guided-form sync
        // listener and the cross-highlight's own cursor listener share the
        // same `updateListener` facet mechanism, each registered
        // independently below.
        extensions: [
          CM.linter(vdLinter, { delay: 400 }),
          CM.lintGutter(),
          // `.cm-content` is `contenteditable`, genuinely reachable by Tab
          // in every real browser without this - but axe's own
          // `scrollable-region-focusable` check does not credit a bare
          // `contenteditable` as "focusable content" for its *ancestor*
          // `.cm-scroller` (which `code-editor.js`'s shared chrome makes
          // `overflow: auto`, #838), only an element that itself carries an
          // explicit `tabindex`. A long ViewDefinition is exactly the
          // document that turns that scroller's overflow on - the SQL pane
          // editors (`sql-editor.js`) share the same chrome but rarely
          // scroll, which is presumably why this has not surfaced there.
          // `contentAttributes` merges every provider's keys (`code-
          // editor.js`'s own `aria-label` included, see its mount, #838) via
          // CodeMirror's own facet combination, so this coexists with it
          // rather than overriding it.
          CM.EditorView.contentAttributes.of({ tabindex: "0" }),
          // `handleCmUpdate` is declared further down this same function -
          // safe to reference here because function declarations (unlike
          // `var`) hoist their implementation too, and this facet is not
          // actually invoked until a real edit, long after `mount()` has
          // finished assigning everything `handleCmUpdate` closes over.
          CM.EditorView.updateListener.of(handleCmUpdate),
          highlightField,
          CM.EditorView.updateListener.of(function (update) {
            if (update.selectionSet || update.docChanged) handleSelectionUpdate();
          }),
        ],
        fold: true,
        wrapperClass: "vd-editor",
        id: "vd-editor",
      });
    }

    // #843's own wiring below needs the guided-form card's container and the
    // shared loop that drives it; either missing (a page fragment other than
    // this one's own template, or the script somehow loading without
    // editor-form.js) leaves the editor above exactly as it is and touches
    // nothing else.
    if (!grid || !EditorForm) return;

    /* ---- the host contract -------------------------------------------
     *
     * CodeMirror mounted: the `EditorView` `code-editor.js` handed back.
     * Not mounted (no bundle, no helper, a construction error): the plain
     * `<textarea>` underneath it, unchanged from before #753 ever existed -
     * `getDoc` reads its value, `setDoc` writes it and fires `input` for the
     * same live-preview wiring a manual edit already triggers.
     */
    var host = view
      ? {
          getDoc: function () {
            return view.state.doc.toString();
          },
          setDoc: function (text) {
            var change = minimalChange(view.state.doc.toString(), text);
            if (change) {
              view.dispatch({
                changes: { from: change.from, to: change.to, insert: change.insert },
                effects: FormOriginEffect.of(true),
              });
            }
            rememberSynced(text);
          },
        }
      : {
          getDoc: function () {
            return textarea.value;
          },
          setDoc: function (text) {
            if (text !== textarea.value) {
              textarea.value = text;
              textarea.dispatchEvent(new Event("input", { bubbles: true }));
            }
            rememberSynced(text);
          },
        };

    formApi = EditorForm.attach(grid, host);

    // #843: hovering or focusing a row lights its node in the editor, and
    // moving the cursor in the editor lights the row back; `grid` (not
    // `document`) is the delegation root, since this page's own
    // cross-highlight has no standalone-editor or Resources-modal sibling to
    // share a document-level listener with the way `editor-sync.js` does.
    // Only wired up once CodeMirror actually mounted - without it there is
    // no editor pane, and thus nothing for a row to highlight; the
    // cursor-side listener is already scoped the same way, registered only
    // inside the `if (CodeEditor && CM)` block above.
    if (view) {
      grid.addEventListener("mouseover", handleRowEnter);
      grid.addEventListener("focusin", handleRowEnter);
      grid.addEventListener("mouseout", handleRowLeave);
      grid.addEventListener("focusout", handleRowLeave);
    }

    if (!view) {
      // No CodeMirror transaction to tag here, so no origin marker - the
      // textarea's own `input` event is the only signal, and `setDoc`
      // (above) fires it too on every form-driven change. The canonical-
      // form comparison below (`lastSynced`, updated by `rememberSynced`
      // before `setDoc` even returns) is what keeps that self-triggered
      // `input` from re-requesting the panel it just came from.
      textarea.addEventListener("input", function () {
        scheduleSync(textarea.value);
      });
    }

    /* `text` is always `#editor-pretty`'s own value here - the server's
     * pretty-printed serialization of the document it just validated as
     * JSON, handed to `host.setDoc` by `editor-form.js` after every
     * mutation. Always parses; the try/catch is cheaper than trusting that
     * invariant blindly across a future change on either side. */
    function rememberSynced(text) {
      try {
        lastSynced = JSON.stringify(JSON.parse(text));
      } catch (alwaysValid) {
        /* see above */
      }
    }

    function chip() {
      return grid.querySelector(".editor-validity");
    }

    /* The validity chip switches to a short, purely client-side "invalid
     * JSON" state - no round trip, no touching the rows underneath. The
     * chip's own prior text/state is cached on first use so a later fix
     * that turns out to match `lastSynced` exactly (no refresh call, see
     * `scheduleSync`) still has something to restore it to. */
    function markChipInvalid() {
      var el = chip();
      if (!el) return;
      if (!invalidChip) {
        invalidChip = {
          text: el.textContent,
          ok: el.classList.contains("editor-validity--ok"),
        };
      }
      el.classList.remove("editor-validity--ok");
      el.textContent = el.dataset.msgInvalidJson || el.textContent;
    }

    function clearChipInvalid() {
      if (!invalidChip) return;
      var el = chip();
      if (el) {
        el.textContent = invalidChip.text;
        if (invalidChip.ok) el.classList.add("editor-validity--ok");
      }
      invalidChip = null;
    }

    /* 600ms after the last keystroke, for a change the form did not itself
     * just cause. Invalid JSON never reaches the server - only the chip
     * changes, client-side, until the text parses again; a parseable text
     * whose canonical form did not actually move is likewise never sent -
     * this comparison is also the only guard the plain-textarea host has,
     * having no transaction to tag an origin onto. */
    function scheduleSync(text) {
      clearTimeout(syncTimer);
      syncTimer = setTimeout(function () {
        var parsed;
        try {
          parsed = JSON.parse(text);
        } catch (invalid) {
          markChipInvalid();
          return;
        }
        clearChipInvalid();
        var canonical = JSON.stringify(parsed);
        if (canonical === lastSynced) return;
        lastSynced = canonical;
        formApi.refresh(text);
      }, 600);
    }

    function handleCmUpdate(update) {
      if (!update.docChanged) return;
      var formOriginated = update.transactions.some(function (tr) {
        return tr.effects.some(function (effect) {
          return effect.is(FormOriginEffect);
        });
      });
      if (formOriginated) return;
      scheduleSync(update.state.doc.toString());
    }
  }

  return { minimalChange: minimalChange, mount: mount };
});
