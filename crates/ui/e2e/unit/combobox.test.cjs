const test = require("node:test");
const assert = require("node:assert/strict");

const combobox = require("../../assets/combobox.js");

test("parses the no-JavaScript textarea contract", () => {
  assert.deepEqual(
    combobox.parseValues("Patient/p-104, Patient/p-205\nPatient/p-306"),
    ["Patient/p-104", "Patient/p-205", "Patient/p-306"],
  );
});

test("trims, drops empty entries, and preserves first-seen order", () => {
  assert.deepEqual(
    combobox.parseValues("\n Patient/p-205 ,,Patient/p-104\n"),
    ["Patient/p-205", "Patient/p-104"],
  );
});

test("deduplicates exact fallback values", () => {
  assert.deepEqual(
    combobox.parseValues("Patient/p-104\nPatient/p-104,Patient/P-104"),
    ["Patient/p-104", "Patient/P-104"],
  );
});

test("parses CR and CRLF lists without losing order or duplicate handling", () => {
  assert.deepEqual(
    combobox.parseValues(" Patient/p-104\rPatient/p-205\r\nPatient/p-104\r\nPatient/p-306 "),
    ["Patient/p-104", "Patient/p-205", "Patient/p-306"],
  );
});

test("single pasted searches discard surrounding separators without validating references", () => {
  for (const value of ["p-1831", "Patient/p-1831", "Ana Rivera", "invalid/id/path"]) {
    for (const separator of ["\n", "\r", "\r\n", ",", ",\n,\r\n"]) {
      assert.deepEqual(combobox.preparePaste("", ` ${separator} ${value} ${separator} `, 0, 0),
        { count: 1, value, caret: value.length });
    }
  }
});

test("pasted lists are classified before deduplication and preserve the composed selection", () => {
  assert.deepEqual(combobox.preparePaste("", "one,one", 0, 0), { count: 2, value: "one,one" });
  assert.deepEqual(combobox.preparePaste("prefix-REPLACE-suffix", "one,two", 7, 14),
    { count: 2, value: "prefix-one,two-suffix" });
  assert.deepEqual(combobox.preparePaste("prefix-REPLACE-suffix", "\none", 7, 14),
    { count: 2, value: "prefix-\none-suffix" });
});

test("a normalized search keeps the insertion caret before the unselected suffix", () => {
  assert.deepEqual(combobox.preparePaste("REPLACE-suffix", "\n one", 0, 7),
    { count: 1, value: "one-suffix", caret: 3 });
  assert.deepEqual(combobox.preparePaste("prefix-REPLACE", "one \r\n", 7, 14),
    { count: 1, value: "prefix-one", caret: 10 });
  assert.deepEqual(combobox.preparePaste("prefix-", "one\n", null, null),
    { count: 1, value: "prefix-one", caret: 10 });
});

test("separator-only paste replaces only the selected range without changing exterior text", () => {
  assert.deepEqual(combobox.preparePaste(" prefix-REPLACE-suffix ", " ,\r\n, ", 8, 15),
    { count: 0, value: " prefix--suffix ", caret: 8 });
  assert.deepEqual(combobox.preparePaste("", "\r\n,, ", 0, 0),
    { count: 0, value: "", caret: 0 });
});

// #842: single-value mode (`data-combobox-max="1"`, the *Add table*
// combobox) — `atCapacity` is the pure decision `add()` uses to know
// whether choosing one more option must first clear the field instead of
// adding a second chip.
test("max=0 (every field before #842) is never at capacity", () => {
  assert.equal(combobox.atCapacity(0, 0), false);
  assert.equal(combobox.atCapacity(5, 0), false);
});

test("max=1 is at capacity once one value is already selected, not before", () => {
  assert.equal(combobox.atCapacity(0, 1), false);
  assert.equal(combobox.atCapacity(1, 1), true);
});

test("a max above 1 only trips once that many are already selected", () => {
  assert.equal(combobox.atCapacity(1, 3), false);
  assert.equal(combobox.atCapacity(3, 3), true);
});

for (const resourceType of ["Patient", "Group"]) {
  test(`${resourceType} manual references use the server logical ID grammar`, () => {
    for (const id of ["one", "A-1.2", "x".repeat(64)]) {
      assert.equal(combobox.validReference(id, resourceType), true);
      assert.equal(combobox.validReference(`${resourceType}/${id}`, resourceType), true);
    }
    for (const value of ["", "a b", "a_b", "é", "x".repeat(65), `${resourceType}/`,
      `${resourceType}/a/b`, "Observation/one", "https://example.test/Patient/one",
      "Andreshttp://example.test/ui/editor?resource=Patient", "patient/one"]) {
      assert.equal(combobox.validReference(value, resourceType), false, value);
    }
  });
}
