const test = require("node:test");
const assert = require("node:assert/strict");

const HfsNumber = require("../../assets/number.js");

test("groups by the given locale, the way the server does", () => {
  assert.equal(HfsNumber.format(70048, { lang: "en" }), "70,048");
  assert.equal(HfsNumber.format(70048, { lang: "de" }), "70.048");
  assert.equal(HfsNumber.format(70048, { lang: "es" }), "70.048");
  assert.equal(HfsNumber.format(1234567, { lang: "de" }), "1.234.567");
});

test("Spanish leaves four-digit figures ungrouped (CLDR minimum grouping)", () => {
  assert.equal(HfsNumber.format(1234, { lang: "es" }), "1234");
  assert.equal(HfsNumber.format(1234, { lang: "de" }), "1.234");
  assert.equal(HfsNumber.format(1234, { lang: "en" }), "1,234");
});

test("decimals take the locale's own separator", () => {
  assert.equal(HfsNumber.format(1.5, { lang: "en" }), "1.5");
  assert.equal(HfsNumber.format(1.5, { lang: "de" }), "1,5");
  assert.equal(HfsNumber.format(1234.5, { lang: "en", maximumFractionDigits: 1 }), "1,234.5");
});

test("numeric strings format; anything else passes through unchanged", () => {
  assert.equal(HfsNumber.format("12345", { lang: "en" }), "12,345");
  assert.equal(HfsNumber.format("abc", { lang: "en" }), "abc");
  assert.equal(HfsNumber.format("", { lang: "en" }), "");
  assert.equal(HfsNumber.format(null, { lang: "en" }), "null");
  assert.equal(HfsNumber.format(Infinity, { lang: "en" }), "Infinity");
});

test("small figures are unchanged", () => {
  assert.equal(HfsNumber.format(0, { lang: "de" }), "0");
  assert.equal(HfsNumber.format(999, { lang: "en" }), "999");
});
