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
