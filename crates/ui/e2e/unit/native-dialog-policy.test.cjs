const test = require("node:test");
const assert = require("node:assert/strict");
const { createNativeDialogPolicy } = require("../pages/native-dialog-policy.cjs");

for (const type of ["confirm", "alert", "prompt"]) {
  test(`an unexpected native ${type} is dismissed and fails the policy`, () => {
    const policy = createNativeDialogPolicy();
    assert.deepEqual(policy.receive({ type, message: "unexpected" }), {
      action: "dismiss", unexpected: true,
    });
    assert.deepEqual(policy.failures(), [`Unexpected native ${type}: unexpected`]);
    // Reading the failures cannot drain them and hide a regression.
    assert.deepEqual(policy.failures(), [`Unexpected native ${type}: unexpected`]);
  });
}

test("beforeunload stays allowed and its armed action is consumed only by beforeunload", () => {
  const policy = createNativeDialogPolicy();
  policy.armBeforeUnload("dismiss");
  assert.equal(policy.receive({ type: "prompt", message: "unexpected" }).unexpected, true);
  assert.equal(policy.receive({ type: "beforeunload", message: "" }).action, "dismiss");
  assert.equal(policy.receive({ type: "beforeunload", message: "" }).action, "accept");
  assert.deepEqual(policy.failures(), ["Unexpected native prompt: unexpected"]);
});

test("an exact prompt expectation passes its response once, never a second dialog", () => {
  const policy = createNativeDialogPolicy();
  policy.expectOnce({ type: "prompt", message: "Rename", action: "accept", promptText: "New name" });
  assert.equal(policy.hasPendingExpectation(), true);
  assert.deepEqual(policy.receive({ type: "prompt", message: "Rename" }), {
    action: "accept", promptText: "New name", unexpected: false,
  });
  assert.equal(policy.hasPendingExpectation(), false);
  assert.deepEqual(policy.failures(), []);
  assert.equal(policy.receive({ type: "prompt", message: "Rename" }).unexpected, true);
  assert.deepEqual(policy.failures(), ["Unexpected native prompt: Rename"]);
});

test("a wrong type or message remains a failure even if the expected prompt follows", () => {
  const policy = createNativeDialogPolicy();
  policy.expectOnce({ type: "prompt", message: "Rename", action: "dismiss" });
  policy.receive({ type: "alert", message: "Rename" });
  policy.receive({ type: "prompt", message: "Wrong question" });
  assert.equal(policy.hasPendingExpectation(), true);
  assert.equal(policy.receive({ type: "prompt", message: "Rename" }).action, "dismiss");
  assert.deepEqual(policy.failures(), [
    "Unexpected native alert: Rename", "Unexpected native prompt: Wrong question",
  ]);
});

test("an unused expectation fails and cannot be silently replaced", () => {
  const policy = createNativeDialogPolicy();
  const expected = { type: "prompt", message: "Rename", action: "accept" };
  policy.expectOnce(expected);
  assert.throws(() => policy.expectOnce(expected), /already pending/);
  assert.deepEqual(policy.failures(), ["Expected native prompt was not shown: Rename"]);
});

test("beforeunload cannot consume an expected prompt", () => {
  const policy = createNativeDialogPolicy();
  policy.expectOnce({ type: "prompt", message: "Rename", action: "dismiss" });
  policy.receive({ type: "beforeunload", message: "" });
  assert.deepEqual(policy.failures(), ["Expected native prompt was not shown: Rename"]);
});
