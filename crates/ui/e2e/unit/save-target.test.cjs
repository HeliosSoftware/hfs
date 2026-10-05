const test = require("node:test");
const assert = require("node:assert/strict");

const saveTarget = require("../../assets/save-target.js");

test("forCreate puts a document that carries an id to that id", () => {
  assert.deepEqual(saveTarget.forCreate("Group", { id: "manual-group" }), {
    method: "PUT",
    url: "/Group/manual-group",
    id: "manual-group",
  });
});

test("forCreate posts when the id is absent, empty or not a string", () => {
  const expected = { method: "POST", url: "/Group", id: null };
  assert.deepEqual(saveTarget.forCreate("Group", {}), expected);
  assert.deepEqual(saveTarget.forCreate("Group", { id: "" }), expected);
  assert.deepEqual(saveTarget.forCreate("Group", { id: 5 }), expected);
});

test("forCreate encodes an id that needs escaping and still uses PUT", () => {
  const target = saveTarget.forCreate("Group", { id: "a b" });
  assert.equal(target.method, "PUT");
  assert.equal(target.url, "/Group/a%20b");
  assert.equal(target.id, "a b");
});

test("isValidId accepts FHIR ids and rejects everything else", () => {
  for (const ok of ["manual-group", "A.1-b", "a".repeat(64)]) {
    assert.equal(saveTarget.isValidId(ok), true, ok);
  }
  for (const bad of ["", "a".repeat(65), "a b", "a/b", null, 5]) {
    assert.equal(saveTarget.isValidId(bad), false, String(bad));
  }
});

test("notice names the target only for a valid id", () => {
  const template = "Will be saved as {target}";
  assert.equal(
    saveTarget.notice("Group", { id: "manual-group" }, template),
    "Will be saved as Group/manual-group",
  );
  assert.equal(saveTarget.notice("Group", { id: "a b" }, template), "");
  assert.equal(saveTarget.notice("Group", {}, template), "");
  assert.equal(saveTarget.notice("Group", null, template), "");
});

test("existsFromStatus is true only for 200", () => {
  assert.equal(saveTarget.existsFromStatus(200), true);
  for (const status of [404, 410, 401, 500, 0]) {
    assert.equal(saveTarget.existsFromStatus(status), false, String(status));
  }
});
