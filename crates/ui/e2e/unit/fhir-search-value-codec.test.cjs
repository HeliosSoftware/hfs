const test = require("node:test");
const assert = require("node:assert/strict");

const codec = require("../../assets/fhir-search-value.js");

function values(wire) {
  return codec.parseAlternatives(wire).alternatives.map((alternative) => alternative.value);
}

test("splits only unescaped commas", () => {
  assert.deepEqual(values("a,b"), ["a", "b"]);
  assert.deepEqual(values("a\\,b"), ["a,b"]);
  assert.deepEqual(values("a\\,b,c"), ["a,b", "c"]);
  assert.deepEqual(values("a\\\\,b"), ["a\\\\", "b"]);
});

test("preserves visible backslash, token and composite escapes", () => {
  assert.deepEqual(values("a\\\\b"), ["a\\\\b"]);
  assert.deepEqual(values("system\\|value"), ["system\\|value"]);
  assert.deepEqual(values("left\\$right"), ["left\\$right"]);
  assert.deepEqual(values("Muñoz\\,García"), ["Muñoz,García"]);
});

test("keeps exact wire alternatives for unchanged inputs", () => {
  const parsed = codec.parseAlternatives("a\\,b,c\\|d");
  assert.deepEqual(
    parsed.alternatives.map((alternative) => alternative.wire),
    ["a\\,b", "c\\|d"],
  );
});

test("serializes edited visual values back to FHIR escaping", () => {
  assert.equal(codec.serializeAlternative("a,b"), "a\\,b");
  assert.equal(codec.serializeAlternative("a\\b"), "a\\\\b");
  assert.equal(codec.serializeAlternative("a\\\\b"), "a\\\\b");
  assert.equal(codec.serializeAlternative("system\\|value"), "system\\|value");
  assert.equal(codec.serializeAlternative("left\\$right"), "left\\$right");
  assert.equal(codec.serializeAlternatives(["a,b", "c"]), "a\\,b,c");
});

test("round-trips adjacent escaped backslashes and structural separators", () => {
  for (const wire of ["a\\\\|b", "a\\\\$b", "a\\\\\\|b", "a\\\\\\$b"]) {
    const visual = values(wire)[0];
    assert.equal(codec.serializeAlternative(visual), wire);
  }
});

test("reports malformed FHIR escapes without dropping their text", () => {
  const trailing = codec.parseAlternatives("a\\");
  assert.equal(trailing.error, "invalid-escape");
  assert.equal(trailing.alternatives[0].error, "trailing-escape");
  assert.equal(trailing.alternatives[0].value, "a\\");
  assert.equal(trailing.alternatives[0].wire, "a\\");

  const unknown = codec.parseAlternatives("a\\x");
  assert.equal(unknown.error, "invalid-escape");
  assert.equal(unknown.alternatives[0].error, "invalid-escape");
  assert.equal(unknown.alternatives[0].value, "a\\x");
  assert.equal(unknown.alternatives[0].wire, "a\\x");
});

test("characterizes empty alternatives", () => {
  assert.deepEqual(values("a,,b"), ["a", "", "b"]);
  assert.deepEqual(values(",a"), ["", "a"]);
  assert.deepEqual(values("a,"), ["a", ""]);
});

test("percent decoding stays outside the FHIR codec", () => {
  const decoded = decodeURIComponent("Comma%5C%2CLiteral");
  assert.deepEqual(values(decoded), ["Comma,Literal"]);
  assert.deepEqual(values("Comma%5C%2CLiteral"), ["Comma%5C%2CLiteral"]);
  assert.equal(decodeURIComponent("%255C"), "%5C");
  assert.equal(decodeURIComponent("%2B"), "+");
});
