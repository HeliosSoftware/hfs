const test = require("node:test");
const assert = require("node:assert/strict");

// #1757: requiring the shared editor helper under Node must not need (or
// define) any browser global; the pure JSON helpers are tested directly.
const hadWindow = typeof globalThis.window;
const hadDocument = typeof globalThis.document;
const globalsBefore = Object.getOwnPropertyNames(globalThis);
const codeEditor = require("../../assets/code-editor.js");
const { formatJson, mapOffset } = codeEditor;

test("requiring the module under Node defines no global", () => {
  assert.equal(hadWindow, "undefined");
  assert.equal(hadDocument, "undefined");
  assert.equal(typeof globalThis.window, "undefined");
  assert.equal(typeof globalThis.document, "undefined");
  assert.deepEqual(
    Object.getOwnPropertyNames(globalThis).filter((k) => !globalsBefore.includes(k)),
    [],
  );
  assert.equal(typeof codeEditor.mount, "function");
  assert.equal(typeof codeEditor.jsonHighlight, "function");
  assert.equal(typeof codeEditor.format, "function");
});

test("formatJson lays a compact document out one member per line", () => {
  const compact = '{"a":1,"b":{"c":[1,2],"d":"x"},"e":true}';
  const expected = [
    "{",
    '  "a": 1,',
    '  "b": {',
    '    "c": [',
    "      1,",
    "      2",
    "    ],",
    '    "d": "x"',
    "  },",
    '  "e": true',
    "}",
  ].join("\n");
  assert.equal(formatJson(compact), expected);
});

test("formatJson is idempotent", () => {
  const once = formatJson('{"a":[1,{"b":null}],"c":{}}');
  assert.equal(formatJson(once), once);
});

test("formatJson copies numbers and strings verbatim", () => {
  const str = '"xA \\" {[,:]} "';
  const out = formatJson('{"a":1.10,"b":1e3,"c":12345678901234567890,"d":' + str + "}");
  assert.ok(out.includes("1.10"));
  assert.ok(out.includes("1e3"));
  assert.ok(out.includes("12345678901234567890"));
  assert.ok(out.includes('"d": ' + str));
});

test("formatJson keeps duplicate keys", () => {
  const out = formatJson('{"a":1,"a":2}');
  assert.equal(out, '{\n  "a": 1,\n  "a": 2\n}');
});

test("formatJson keeps empty containers on one line", () => {
  const out = formatJson('{"a":{},"b":[]}');
  assert.ok(out.includes('"a": {}\n') || out.includes('"a": {},'));
  assert.ok(out.includes('"b": []'));
  assert.equal(formatJson("{ }"), "{}");
  assert.equal(formatJson("[ \n ]"), "[]");
});

test("formatJson returns null for invalid input", () => {
  assert.equal(formatJson('{"a":1,}'), null);
  assert.equal(formatJson('{"a":"open'), null);
  assert.equal(formatJson(""), null);
  assert.equal(formatJson("   \n "), null);
});

test("formatJson normalises CRLF and tabs to LF and spaces", () => {
  const out = formatJson('{\r\n\t"a":\t1,\r\n\t"b": 2\r\n}\r\n');
  assert.equal(out, '{\n  "a": 1,\n  "b": 2\n}');
  assert.ok(!out.includes("\r"));
  assert.ok(!out.includes("\t"));
});

test("formatJson handles array and scalar roots", () => {
  assert.equal(formatJson("[1,2]"), "[\n  1,\n  2\n]");
  assert.equal(formatJson(" 42 "), "42");
  assert.equal(formatJson(' "x" '), '"x"');
});

test("mapOffset keeps the cursor on the same significant character", () => {
  const before = '{"alpha":1,"beta":"hello world"}';
  const after = formatJson(before);
  assert.equal(mapOffset(before, after, 0), 0);
  assert.equal(mapOffset(before, after, before.length), after.length);

  const afterKey = before.indexOf('"alpha"') + '"alpha"'.length;
  const mapped = mapOffset(before, after, afterKey);
  assert.equal(after.slice(0, mapped).endsWith('"alpha"'), true);
  assert.equal(after.slice(mapped, mapped + 1), ":");

  const inString = before.indexOf("hello") + 3;
  const mappedIn = mapOffset(before, after, inString);
  assert.equal(after.slice(mappedIn - 3, mappedIn + 8), "hello world");
});
