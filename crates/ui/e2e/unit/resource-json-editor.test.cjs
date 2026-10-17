const test = require("node:test");
const assert = require("node:assert/strict");

const { issueDiagnostics, sameDocument } = require("../../assets/resource-json-editor.js");

// #1756: the server's validation issues become amber line markers. The
// function is pure: `resolve(path)` stands in for the editor's syntax tree.

const ranges = { gender: { from: 10, to: 25 }, name: { from: 30, to: 60 } };
const resolve = (path) => ranges[path] || null;

test("a path that resolves becomes one warning on its range, message prefixed by the path", () => {
  const out = issueDiagnostics([{ path: "gender", message: "bad code" }], resolve);
  assert.deepEqual(out, [
    { from: 10, to: 25, severity: "warning", message: "gender: bad code", source: "fhir" },
  ]);
});

test("a path that does not resolve falls back to its nearest resolvable ancestor", () => {
  const out = issueDiagnostics([{ path: "name.0.family", message: "missing" }], resolve);
  assert.equal(out.length, 1);
  assert.equal(out[0].from, 30);
  assert.equal(out[0].to, 60);
  assert.equal(out[0].message, "name.0.family: missing");
});

test("the root path marks the first character and carries no prefix", () => {
  const out = issueDiagnostics([{ path: "", message: "whole document" }], resolve);
  assert.deepEqual(out, [
    { from: 0, to: 1, severity: "warning", message: "whole document", source: "fhir" },
  ]);
});

test("a path with no resolvable ancestor lands on the first character", () => {
  const out = issueDiagnostics([{ path: "nowhere.0", message: "m" }], resolve);
  assert.equal(out[0].from, 0);
  assert.equal(out[0].to, 1);
});

test("identical issues collapse into one diagnostic", () => {
  const issue = { path: "gender", message: "bad code" };
  assert.equal(issueDiagnostics([issue, { ...issue }], resolve).length, 1);
  assert.equal(issueDiagnostics([issue, { path: "gender", message: "other" }], resolve).length, 2);
});

test("malformed input yields no diagnostics", () => {
  assert.deepEqual(issueDiagnostics(null, resolve), []);
  assert.deepEqual(issueDiagnostics([null, { path: "gender" }], resolve), []);
});

test("sameDocument ignores object key order but not values or array order", () => {
  assert.equal(sameDocument('{"a":1,"b":{"c":[1,2],"d":null}}', '{ "b": {"d": null, "c": [1,2]}, "a": 1 }'), true);
  assert.equal(sameDocument('{"a":1}', '{"a":2}'), false);
  assert.equal(sameDocument('{"a":[1,2]}', '{"a":[2,1]}'), false);
  assert.equal(sameDocument('{"a":1}', '{"a":1,"b":2}'), false);
  assert.equal(sameDocument('{"a":1}', "{broken"), false);
});
