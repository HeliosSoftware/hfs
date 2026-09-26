const test = require("node:test");
const assert = require("node:assert/strict");

const unsaved = require("../../assets/unsaved.js");

// #1240: the shared unsaved-changes tracker's comparison — trimmed values,
// JSON compared by canonical content rather than letter by letter (the same
// rule editor-pair.js already uses for its own JSON<->form sync). `serialize`
// needs a real DOM (`form.elements`), so it is exercised in Playwright
// instead; these tests only need `normalize`, plain strings, no window/
// document at all.

test("normalize trims leading and trailing whitespace", () => {
  assert.equal(unsaved.normalize("  a b  "), "a b");
  assert.equal(unsaved.normalize("  "), "");
});

test("normalize compares JSON by content", () => {
  const spaced = '{ "a": 1,\n "b": [1, 2] }';
  const compact = '{"a":1,"b":[1,2]}';
  assert.equal(unsaved.normalize(spaced), unsaved.normalize(compact));
});

test("normalize keeps invalid JSON as text", () => {
  assert.equal(unsaved.normalize('{"a":'), '{"a":');
});

test("normalize leaves non-JSON text intact", () => {
  assert.equal(unsaved.normalize("SELECT 1"), "SELECT 1");
});
