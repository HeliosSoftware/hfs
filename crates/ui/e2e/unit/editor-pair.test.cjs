const test = require("node:test");
const assert = require("node:assert/strict");

const editorPair = require("../../assets/editor-pair.js");

// #843 (extracted onto editor-pair.js in #840): the guided form pushes its
// edits into CodeMirror as one minimal transaction (common-prefix/
// common-suffix diff), so the caret, the scroll position, and the undo
// history all behave like a targeted manual edit instead of a
// full-document replace.

test("identical text produces no change (no transaction)", () => {
  assert.equal(editorPair.minimalChange("same", "same"), null);
  assert.equal(editorPair.minimalChange("", ""), null);
});

test("an insertion at the end is a single trailing insert", () => {
  assert.deepEqual(editorPair.minimalChange("abc", "abcdef"), {
    from: 3,
    to: 3,
    insert: "def",
  });
});

test("an insertion at the start is a single leading insert", () => {
  assert.deepEqual(editorPair.minimalChange("def", "abcdef"), {
    from: 0,
    to: 0,
    insert: "abc",
  });
});

test("a replacement in the middle keeps the untouched prefix and suffix out of the range", () => {
  assert.deepEqual(editorPair.minimalChange("abcXdef", "abcYYdef"), {
    from: 3,
    to: 4,
    insert: "YY",
  });
});

test("a deletion is an empty insert over the removed range", () => {
  assert.deepEqual(editorPair.minimalChange("abcXYZdef", "abcdef"), {
    from: 3,
    to: 6,
    insert: "",
  });
});

test("replacing the whole document (no shared prefix or suffix) covers the full range", () => {
  assert.deepEqual(editorPair.minimalChange("abc", "xyz"), {
    from: 0,
    to: 3,
    insert: "xyz",
  });
});

test("a shared prefix and suffix that overlap in the shorter text do not double-count", () => {
  // "aaa" -> "aa": every character in "aa" matches as both a prefix and (if
  // unguarded) a suffix of "aaa" - the shared region must not be counted
  // twice, which would put `to` before `from`.
  const change = editorPair.minimalChange("aaa", "aa");
  assert.ok(change.from <= change.to, `from (${change.from}) must not exceed to (${change.to})`);
  // Re-applying the change must reproduce the target text exactly.
  const applied = "aaa".slice(0, change.from) + change.insert + "aaa".slice(change.to);
  assert.equal(applied, "aa");
});

test("pretty-printing a document (whitespace-only change) is still a minimal range, not a full replace", () => {
  const before = '{"a":1}';
  const after = '{\n  "a": 1\n}';
  const change = editorPair.minimalChange(before, after);
  // The literal "1" in the middle is untouched text on both sides, so the
  // diff must not span the entire document even though nearly every
  // character differs once whitespace is inserted around it.
  assert.ok(change.from > 0 || change.to < before.length, "not a full-document replace");
  const applied = before.slice(0, change.from) + change.insert + before.slice(change.to);
  assert.equal(applied, after);
});

// #1756: `rangeOfPath` resolves a dotted path to the key+value range in a
// real CodeMirror state, built from the vendored bundle the way the
// bundle test loads it.
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

function loadCodeMirror() {
  const source = fs.readFileSync(path.join(__dirname, "../../assets/vendor/codemirror.bundle.js"), "utf8");
  const sandbox = { window: {} };
  vm.runInContext(source, vm.createContext(sandbox), { filename: "codemirror.bundle.js" });
  return sandbox.window.HfsCodeMirror;
}

test("rangeOfPath covers key and value, and is null for an absent path", () => {
  const CM = loadCodeMirror();
  const text = '{\n  "resourceType": "Patient",\n  "gender": "M",\n  "name": [{ "family": "X" }]\n}';
  const state = CM.EditorState.create({ doc: text, extensions: [CM.json()] });
  const gender = editorPair.rangeOfPath(state, "gender", CM);
  assert.equal(text.slice(gender.from, gender.to), '"gender": "M"');
  const family = editorPair.rangeOfPath(state, "name.0.family", CM);
  assert.equal(text.slice(family.from, family.to), '"family": "X"');
  const item = editorPair.rangeOfPath(state, "name.0", CM);
  assert.equal(text.slice(item.from, item.to), '{ "family": "X" }');
  assert.equal(editorPair.rangeOfPath(state, "birthDate", CM), null);
});
