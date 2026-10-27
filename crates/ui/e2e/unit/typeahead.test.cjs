const test = require("node:test");
const assert = require("node:assert/strict");

const typeahead = require("../../assets/typeahead.js");

const opt = (value, hint) => ({ value, hint: hint || "" });
const values = (list) => list.map((o) => o.value);

test("normalize lower-cases and drops hyphens and underscores", () => {
  assert.equal(typeahead.normalize("_lastUpdated"), "lastupdated");
  assert.equal(typeahead.normalize("address-city"), "addresscity");
});

test("an empty query returns a copy in the original order", () => {
  const options = [opt("b"), opt("a"), opt("c")];
  const result = typeahead.filterOptions(options, "  ");
  assert.deepEqual(result, options);
  assert.notEqual(result, options);
});

test("prefix matches come before contains matches, keeping order", () => {
  const options = [opt("dead"), opt("address"), opt("address-city"), opt("name")];
  assert.deepEqual(values(typeahead.filterOptions(options, "ad")), [
    "address",
    "address-city",
    "dead",
  ]);
});

test("a trailing fragment finds the option", () => {
  const options = [opt("name"), opt("address-city")];
  assert.deepEqual(values(typeahead.filterOptions(options, "city")), ["address-city"]);
});

test("matching ignores case and separators", () => {
  const options = [opt("birthdate"), opt("_lastUpdated"), opt("address-city")];
  assert.deepEqual(values(typeahead.filterOptions(options, "BIRTH")), ["birthdate"]);
  assert.deepEqual(values(typeahead.filterOptions(options, "lastupdated")), ["_lastUpdated"]);
  assert.deepEqual(values(typeahead.filterOptions(options, "addresscity")), ["address-city"]);
});

test("hint matches follow name matches without duplicates", () => {
  const options = [
    opt("birthdate", "date"),
    opt("death-date", "date"),
    opt("_lastUpdated", "date"),
    opt("name", "string"),
  ];
  assert.deepEqual(values(typeahead.filterOptions(options, "date")), [
    "birthdate",
    "death-date",
    "_lastUpdated",
  ]);
  const mixed = [opt("zeta", "date"), opt("update", "token"), opt("date-x", "string")];
  assert.deepEqual(values(typeahead.filterOptions(mixed, "date")), ["date-x", "update", "zeta"]);
});

test("no match returns an empty list", () => {
  assert.deepEqual(typeahead.filterOptions([opt("name"), opt("birthdate")], "zzz"), []);
});

test("a separator-only query matches literally", () => {
  const options = [opt("_lastUpdated"), opt("name"), opt("address-city"), opt("_id")];
  assert.deepEqual(values(typeahead.filterOptions(options, "_")), ["_lastUpdated", "_id"]);
});

test("matchRange returns the literal range or null", () => {
  assert.deepEqual(typeahead.matchRange("address-city", "CITY"), [8, 12]);
  assert.equal(typeahead.matchRange("address-city", "addresscity"), null);
});
