const test = require("node:test");
const assert = require("node:assert/strict");

const editorAdd = require("../../assets/editor-add.js");

// #1239: `matches`, `parentPath`, and `leafName` are the add-picker's own
// pure rules, exported so they can be exercised without a DOM.
// `capturePickers` and `restorePickers` need real elements, so they stay
// covered by the existing Playwright specs over the three hosts instead.

test("an empty needle matches everything", () => {
  assert.equal(editorAdd.matches("birthDate", ""), true);
  assert.equal(editorAdd.matches("", ""), true);
});

test("a matching substring is found regardless of position", () => {
  assert.equal(editorAdd.matches("birthDate", "date"), true);
  assert.equal(editorAdd.matches("birthDate", "birth"), true);
  assert.equal(editorAdd.matches("birthDate", "hDa"), true);
});

test("the match is case-insensitive", () => {
  assert.equal(editorAdd.matches("birthDate", "DATE"), true);
  assert.equal(editorAdd.matches("BirthDate", "date"), true);
});

test("no match returns false", () => {
  assert.equal(editorAdd.matches("birthDate", "xyz"), false);
});

test("parentPath drops a trailing index and the last path segment", () => {
  assert.equal(editorAdd.parentPath("birthDate"), "");
  assert.equal(editorAdd.parentPath("name[1]"), "");
  assert.equal(editorAdd.parentPath("name[0].family"), "name[0]");
  assert.equal(editorAdd.parentPath("name[0].given[2]"), "name[0]");
  assert.equal(editorAdd.parentPath("extension[0]"), "");
  assert.equal(editorAdd.parentPath(""), "");
  // The editor's own dotted spelling (`crates/ui/src/editor.rs`).
  assert.equal(editorAdd.parentPath("name.1"), "");
  assert.equal(editorAdd.parentPath("extension.0"), "");
  assert.equal(editorAdd.parentPath("name.0.family"), "name.0");
  assert.equal(editorAdd.parentPath("name.0.given.2"), "name.0");
  assert.equal(editorAdd.parentPath("name.0.extension.0"), "name.0");
});

test("leafName drops a trailing index and keeps the last path segment", () => {
  assert.equal(editorAdd.leafName("birthDate"), "birthDate");
  assert.equal(editorAdd.leafName("name[1]"), "name");
  assert.equal(editorAdd.leafName("name[0].family"), "family");
  assert.equal(editorAdd.leafName("valueString"), "valueString");
  assert.equal(editorAdd.leafName("extension[0]"), "extension");
  assert.equal(editorAdd.leafName("name.1"), "name");
  assert.equal(editorAdd.leafName("extension.0"), "extension");
  assert.equal(editorAdd.leafName("name.0.family"), "family");
  assert.equal(editorAdd.leafName("name.0.given.2"), "given");
});
