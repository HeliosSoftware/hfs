const test = require("node:test");
const assert = require("node:assert/strict");

const vdEditor = require("../../assets/vd-editor.js");

// #843: the guided form pushes its edits into CodeMirror as one minimal
// transaction (common-prefix/common-suffix diff), so the caret, the scroll
// position, and the undo history all behave like a targeted manual edit
// instead of a full-document replace.

test("identical text produces no change (no transaction)", () => {
  assert.equal(vdEditor.minimalChange("same", "same"), null);
  assert.equal(vdEditor.minimalChange("", ""), null);
});

test("an insertion at the end is a single trailing insert", () => {
  assert.deepEqual(vdEditor.minimalChange("abc", "abcdef"), {
    from: 3,
    to: 3,
    insert: "def",
  });
});

test("an insertion at the start is a single leading insert", () => {
  assert.deepEqual(vdEditor.minimalChange("def", "abcdef"), {
    from: 0,
    to: 0,
    insert: "abc",
  });
});

test("a replacement in the middle keeps the untouched prefix and suffix out of the range", () => {
  assert.deepEqual(vdEditor.minimalChange("abcXdef", "abcYYdef"), {
    from: 3,
    to: 4,
    insert: "YY",
  });
});

test("a deletion is an empty insert over the removed range", () => {
  assert.deepEqual(vdEditor.minimalChange("abcXYZdef", "abcdef"), {
    from: 3,
    to: 6,
    insert: "",
  });
});

test("replacing the whole document (no shared prefix or suffix) covers the full range", () => {
  assert.deepEqual(vdEditor.minimalChange("abc", "xyz"), {
    from: 0,
    to: 3,
    insert: "xyz",
  });
});

test("a shared prefix and suffix that overlap in the shorter text do not double-count", () => {
  // "aaa" -> "aa": every character in "aa" matches as both a prefix and (if
  // unguarded) a suffix of "aaa" - the shared region must not be counted
  // twice, which would put `to` before `from`.
  const change = vdEditor.minimalChange("aaa", "aa");
  assert.ok(change.from <= change.to, `from (${change.from}) must not exceed to (${change.to})`);
  // Re-applying the change must reproduce the target text exactly.
  const applied = "aaa".slice(0, change.from) + change.insert + "aaa".slice(change.to);
  assert.equal(applied, "aa");
});

test("pretty-printing a document (whitespace-only change) is still a minimal range, not a full replace", () => {
  const before = '{"a":1}';
  const after = '{\n  "a": 1\n}';
  const change = vdEditor.minimalChange(before, after);
  // The literal "1" in the middle is untouched text on both sides, so the
  // diff must not span the entire document even though nearly every
  // character differs once whitespace is inserted around it.
  assert.ok(change.from > 0 || change.to < before.length, "not a full-document replace");
  const applied = before.slice(0, change.from) + change.insert + before.slice(change.to);
  assert.equal(applied, after);
});

// #821: the completion pure helpers - the per-`detail` key skeleton and the
// comma placement a new-key insertion needs, in each of the three positions
// the spec calls out (first, intermediate, last property).

test("skeletonForDetail returns one representative empty value per key kind", () => {
  assert.equal(vdEditor.skeletonForDetail("string"), '""');
  assert.equal(vdEditor.skeletonForDetail("boolean"), "true");
  assert.equal(vdEditor.skeletonForDetail("number"), "0");
  assert.equal(vdEditor.skeletonForDetail("string[]"), '[""]');
  assert.equal(vdEditor.skeletonForDetail("object"), "{}");
  assert.equal(vdEditor.skeletonForDetail("object[]"), "[{}]");
  // "other", and anything this client does not otherwise recognize.
  assert.equal(vdEditor.skeletonForDetail("other"), "null");
  assert.equal(vdEditor.skeletonForDetail(undefined), "null");
});

test("skeletonCursorOffset lands inside an empty string/object, after anything else", () => {
  assert.equal(vdEditor.skeletonCursorOffset('""'), 1);
  assert.equal(vdEditor.skeletonCursorOffset('[""]'), 2);
  assert.equal(vdEditor.skeletonCursorOffset("{}"), 1);
  assert.equal(vdEditor.skeletonCursorOffset("[{}]"), 2);
  assert.equal(vdEditor.skeletonCursorOffset("true"), 4);
  assert.equal(vdEditor.skeletonCursorOffset("0"), 1);
  assert.equal(vdEditor.skeletonCursorOffset("null"), 4);
});

test("classifyObjectGap: first property (empty object) needs no comma either side", () => {
  // `{}`, cursor right after "{" - beforeIndex -1, nothing typed yet.
  const gap = vdEditor.classifyObjectGap(["{", "}"], -1);
  assert.deepEqual(gap, { leadingComma: false, trailingComma: false });
});

test("classifyObjectGap: first property ahead of an existing one needs a trailing comma only", () => {
  // `{}` with one existing property still to come: `{|"b":2}`.
  const gap = vdEditor.classifyObjectGap(["{", "Property", "}"], 0);
  assert.deepEqual(gap, { leadingComma: false, trailingComma: true });
});

test("classifyObjectGap: intermediate gap right after a comma needs no comma of its own", () => {
  // `{"a":1,|"b":2}` - the "," already separates the new key from "a".
  const gap = vdEditor.classifyObjectGap(["{", "Property", ",", "Property", "}"], 2);
  assert.deepEqual(gap, { leadingComma: false, trailingComma: true });
});

test("classifyObjectGap: intermediate gap with a missing comma on both sides needs both", () => {
  // `{"a":1 |"b":2}` - no "," was ever typed between "a" and "b"; the child
  // right before the gap is "a"'s own Property, at index 1.
  const gap = vdEditor.classifyObjectGap(["{", "Property", "Property", "}"], 1);
  assert.deepEqual(gap, { leadingComma: true, trailingComma: true });
});

test("classifyObjectGap: last property after an existing comma needs no comma", () => {
  // `{"a":1,|}` - the trailing comma from "a" already separates it.
  const gap = vdEditor.classifyObjectGap(["{", "Property", ",", "}"], 2);
  assert.deepEqual(gap, { leadingComma: false, trailingComma: false });
});

test("classifyObjectGap: last property with no trailing comma yet needs a leading one", () => {
  // `{"a":1|}` - no "," was typed after "a" yet.
  const gap = vdEditor.classifyObjectGap(["{", "Property", "}"], 1);
  assert.deepEqual(gap, { leadingComma: true, trailingComma: false });
});

test("classifyObjectGap: right after the closing brace is not a key position", () => {
  const gap = vdEditor.classifyObjectGap(["{", "Property", "}"], 2);
  assert.equal(gap, null);
});

// objectGapAt: `classifyObjectGap`'s real caller, fed the JSON `Object`'s
// actual direct children (`{name, from, to}`, matching a live `SyntaxNode`)
// rather than a hand-picked name list - `lezer-json`'s own error recovery
// splices a zero-width `"⚠"` node into exactly the gap right after a `","`
// (or between two `Property`s with a missing one), at the same offset as
// whichever real token follows it, whenever the object does not simply end
// right there. Left unfiltered, that placeholder's own `.to <= pos` beat
// the real `","`/`Property` one slot earlier and made every one of these
// positions resolve to "not a key position" - the gap right after a comma
// (in a document CodeMirror had already fully parsed, not just a
// mid-keystroke error state) never offered completion at all.

test("objectGapAt: right after a comma, no whitespace before the closing brace", () => {
  // `{"name":"id",|}` - the exact shape `lezer-json` produces for this.
  const children = [
    { name: "{", from: 0, to: 1 },
    { name: "Property", from: 1, to: 12 },
    { name: ",", from: 12, to: 13 },
    { name: "⚠", from: 13, to: 13 },
    { name: "}", from: 13, to: 14 },
  ];
  assert.deepEqual(vdEditor.objectGapAt(children, 13), { leadingComma: false, trailingComma: false });
});

test("objectGapAt: right after a comma, with whitespace before the closing brace", () => {
  // `{"name":"id", |}` - the error node sits at the brace's own offset,
  // past the space, not at `pos` itself.
  const children = [
    { name: "{", from: 0, to: 1 },
    { name: "Property", from: 1, to: 12 },
    { name: ",", from: 12, to: 13 },
    { name: "⚠", from: 14, to: 14 },
    { name: "}", from: 14, to: 15 },
  ];
  assert.deepEqual(vdEditor.objectGapAt(children, 14), { leadingComma: false, trailingComma: false });
});

test("objectGapAt: a missing comma between two properties still needs both", () => {
  // `{"a":1"b":2}` - the error node sits between the two `Property`s, at
  // the same offset the second one starts.
  const children = [
    { name: "{", from: 0, to: 1 },
    { name: "Property", from: 1, to: 6 },
    { name: "⚠", from: 6, to: 6 },
    { name: "Property", from: 6, to: 11 },
    { name: "}", from: 11, to: 12 },
  ];
  assert.deepEqual(vdEditor.objectGapAt(children, 6), { leadingComma: true, trailingComma: true });
});

test("objectGapAt: right after the closing brace is still not a key position", () => {
  const children = [
    { name: "{", from: 0, to: 1 },
    { name: "Property", from: 1, to: 12 },
    { name: "}", from: 12, to: 13 },
  ];
  assert.equal(vdEditor.objectGapAt(children, 13), null);
});

test("buildKeyInsertion: first property in an empty object needs no comma", () => {
  const built = vdEditor.buildKeyInsertion("name", '""', "", "");
  assert.equal(built.text, '"name": ""');
  // Cursor lands inside the empty string, right after its opening quote.
  assert.equal(built.text.slice(0, built.cursor), '"name": "');
});

test("buildKeyInsertion: intermediate property needs a trailing comma", () => {
  const built = vdEditor.buildKeyInsertion("column", "[{}]", "", ",");
  assert.equal(built.text, '"column": [{}],');
  assert.equal(built.text.slice(0, built.cursor), '"column": [{');
});

test("buildKeyInsertion: last property needs a leading comma", () => {
  const built = vdEditor.buildKeyInsertion("forEach", '""', ",", "");
  assert.equal(built.text, ',"forEach": ""');
  assert.equal(built.text.slice(0, built.cursor), ',"forEach": "');
});

test("buildKeyInsertion: a boolean skeleton has no inner cursor position - it lands after", () => {
  const built = vdEditor.buildKeyInsertion("collection", "true", "", "");
  assert.equal(built.text, '"collection": true');
  assert.equal(built.cursor, built.text.length);
});

// #821: FHIRPath completion's char-offset/UTF-16 conversion (`cursor`/`from`
// in the wire contract are Unicode code points, CodeMirror positions are
// UTF-16 code units - identical for ASCII/BMP text, not for an astral
// character like an emoji).

test("codePointOffset matches the UTF-16 offset for ASCII text", () => {
  assert.equal(vdEditor.codePointOffset("Patient.name", 7), 7);
});

test("codePointOffset counts an astral character (surrogate pair) as one code point", () => {
  // "%c" + a single astral emoji + "onst" - the emoji is 2 UTF-16 code units
  // but 1 code point.
  const text = "%c\u{1F600}onst";
  // Past the emoji entirely (5 UTF-16 units in: "%c" + the pair + "o").
  assert.equal(vdEditor.codePointOffset(text, 5), 4);
});

test("utf16OffsetForCodePoints is the inverse of codePointOffset", () => {
  const text = "%c\u{1F600}onst";
  const codePoints = vdEditor.codePointOffset(text, text.length);
  assert.equal(vdEditor.utf16OffsetForCodePoints(text, codePoints), text.length);
  assert.equal(vdEditor.utf16OffsetForCodePoints(text, 3), 4); // "%", "c", the emoji.
});

// #821: the pure half of applying a lint diagnostic's `Fix` by pointer -
// `removeKeyRange` (remove-key), `stringContentRange` (rename-key and
// set-string both replace only what is inside a string's quotes), and
// `escapeJsonStringContent` (set-string's own escaping). Node stubs below
// carry only what each function actually reads (`from`/`to`/`name`/
// `prevSibling`/`nextSibling`/`parent.firstChild.to`) - never a real parsed
// tree, matching `classifyObjectGap`'s own plain-data testing style above.
// Every `removeKeyRange` case is verified by actually slicing the deletion
// range out of real text and asserting the untouched properties survive
// byte for byte, per the ticket's own acceptance bar.

function propertyNodeIn(text, key, openBraceTo, prevComma, nextComma) {
  const from = text.indexOf('"' + key + '"');
  const closeQuote = text.indexOf('"', from + 1);
  const colon = text.indexOf(":", closeQuote);
  let valueEnd = colon + 1;
  while (/\s/.test(text[valueEnd])) valueEnd++;
  while (valueEnd < text.length && !",}\n".includes(text[valueEnd])) valueEnd++;
  return {
    from: from,
    to: valueEnd,
    prevSibling: prevComma ? { name: ",", from: prevComma[0], to: prevComma[1] } : null,
    nextSibling: nextComma ? { name: ",", from: nextComma[0], to: nextComma[1] } : null,
    parent: { firstChild: { to: openBraceTo } },
  };
}

test("removeKeyRange: the first of three properties leaves the other two untouched, byte for byte", () => {
  const text = '{\n  "a": 1,\n  "b": 2,\n  "c": 3\n}';
  const openBraceTo = text.indexOf("{") + 1;
  const commaAfterA = [text.indexOf(",", text.indexOf('"a"')), text.indexOf(",", text.indexOf('"a"')) + 1];
  const property = propertyNodeIn(text, "a", openBraceTo, null, commaAfterA);
  const range = vdEditor.removeKeyRange(property);
  const applied = text.slice(0, range.from) + text.slice(range.to);
  assert.equal(applied, '{\n  "b": 2,\n  "c": 3\n}');
});

test("removeKeyRange: an intermediate property (both a leading and a trailing comma) leaves its neighbors' indentation unchanged", () => {
  const text = '{\n  "a": 1,\n  "b": 2,\n  "c": 3\n}';
  const openBraceTo = text.indexOf("{") + 1;
  const commaAfterA = text.indexOf(",", text.indexOf('"a"'));
  const commaAfterB = text.indexOf(",", text.indexOf('"b"'));
  const property = propertyNodeIn(text, "b", openBraceTo, [commaAfterA, commaAfterA + 1], [
    commaAfterB,
    commaAfterB + 1,
  ]);
  const range = vdEditor.removeKeyRange(property);
  const applied = text.slice(0, range.from) + text.slice(range.to);
  assert.equal(applied, '{\n  "a": 1,\n  "c": 3\n}');
});

test("removeKeyRange: the last property (no trailing comma of its own) also drops the now-unneeded comma before it", () => {
  const text = '{\n  "a": 1,\n  "b": 2,\n  "c": 3\n}';
  const openBraceTo = text.indexOf("{") + 1;
  const commaAfterB = text.indexOf(",", text.indexOf('"b"'));
  const property = propertyNodeIn(text, "c", openBraceTo, [commaAfterB, commaAfterB + 1], null);
  const range = vdEditor.removeKeyRange(property);
  const applied = text.slice(0, range.from) + text.slice(range.to);
  assert.equal(applied, '{\n  "a": 1,\n  "b": 2\n}');
});

test("removeKeyRange: the object's only property leaves a well-formed empty object", () => {
  const text = '{\n  "a": 1\n}';
  const openBraceTo = text.indexOf("{") + 1;
  const property = propertyNodeIn(text, "a", openBraceTo, null, null);
  const range = vdEditor.removeKeyRange(property);
  const applied = text.slice(0, range.from) + text.slice(range.to);
  assert.equal(applied, "{\n}");
});

test("removeKeyRange: indentation style (4 spaces, no shared prefix with the sibling) does not matter - only sibling positions do", () => {
  const text = '{\n    "first": true,\n    "second": false\n}';
  const openBraceTo = text.indexOf("{") + 1;
  const commaAfterFirst = text.indexOf(",", text.indexOf('"first"'));
  const property = propertyNodeIn(text, "first", openBraceTo, null, [commaAfterFirst, commaAfterFirst + 1]);
  const range = vdEditor.removeKeyRange(property);
  const applied = text.slice(0, range.from) + text.slice(range.to);
  assert.equal(applied, '{\n    "second": false\n}');
});

test("removeKeyRange: a single-line object with no whitespace around commas removes cleanly", () => {
  const text = '{"a":1,"b":2,"c":3}';
  const openBraceTo = text.indexOf("{") + 1;
  const commaAfterA = text.indexOf(",");
  const commaAfterB = text.indexOf(",", commaAfterA + 1);
  const property = propertyNodeIn(text, "b", openBraceTo, [commaAfterA, commaAfterA + 1], [
    commaAfterB,
    commaAfterB + 1,
  ]);
  const range = vdEditor.removeKeyRange(property);
  const applied = text.slice(0, range.from) + text.slice(range.to);
  assert.equal(applied, '{"a":1,"c":3}');
});

test("stringContentRange: strips the surrounding quotes from a value string (set-string's target)", () => {
  const text = '"select(\'a\' = 1)"';
  const range = vdEditor.stringContentRange({ from: 0, to: text.length });
  assert.equal(text.slice(range.from, range.to), "select('a' = 1)");
});

test("stringContentRange: strips the surrounding quotes from a property-name string (rename-key's target)", () => {
  const text = '"columns"';
  const range = vdEditor.stringContentRange({ from: 0, to: text.length });
  assert.equal(text.slice(range.from, range.to), "columns");
  // Splicing the corrected key straight in, exactly as `renameKeyChange` does.
  const renamed = text.slice(0, range.from) + "column" + text.slice(range.to);
  assert.equal(renamed, '"column"');
});

test("stringContentRange: an empty string's content range does not invert (from does not exceed to)", () => {
  const range = vdEditor.stringContentRange({ from: 5, to: 7 }); // `""`
  assert.equal(range.from, 6);
  assert.equal(range.to, 6);
});

test("escapeJsonStringContent: leaves plain text untouched", () => {
  assert.equal(vdEditor.escapeJsonStringContent("Patient.name"), "Patient.name");
});

test("escapeJsonStringContent: escapes a double quote", () => {
  assert.equal(vdEditor.escapeJsonStringContent('say "hi"'), 'say \\"hi\\"');
});

test("escapeJsonStringContent: escapes a backslash before it would otherwise be read as one of its own escapes", () => {
  assert.equal(vdEditor.escapeJsonStringContent("C:\\path"), "C:\\\\path");
});

test("escapeJsonStringContent: backslashes are escaped first, so an escaped quote is not re-escaped", () => {
  // Naive quote-then-backslash ordering would turn `\"` into `\\\"` (an
  // escaped backslash followed by an unescaped quote) instead of `\\\"`
  // read as one escaped backslash plus one escaped quote.
  assert.equal(vdEditor.escapeJsonStringContent('\\"'), '\\\\\\"');
});
