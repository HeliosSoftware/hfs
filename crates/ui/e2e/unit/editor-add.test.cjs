const test = require("node:test");
const assert = require("node:assert/strict");

const editorAdd = require("../../assets/editor-add.js");

// #1239: `matches` is the add-picker's own typeahead rule, exported as a
// pure function so it can be exercised without a DOM. `capturePickers` and
// `restorePickers` need real elements, so they stay covered by the existing
// Playwright specs over the three hosts instead.

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
