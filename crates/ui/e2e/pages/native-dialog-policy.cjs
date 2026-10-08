"use strict";

// The page fixture and the fast tests use this same policy. A recorded native
// dialog stays a failure even when a spec drains its informational dialog log.
function createNativeDialogPolicy() {
  let beforeUnloadAction = null;
  let expected = null;
  const unexpected = [];

  return {
    armBeforeUnload(action) {
      beforeUnloadAction = action;
    },

    expectOnce(expectation) {
      if (expected) throw new Error("A native dialog expectation is already pending");
      if (
        !["confirm", "alert", "prompt"].includes(expectation.type) ||
        typeof expectation.message !== "string" ||
        !["accept", "dismiss"].includes(expectation.action)
      ) {
        throw new Error("Expected native dialog requires an exact type, message and action");
      }
      expected = { ...expectation };
    },

    receive(dialog) {
      if (dialog.type === "beforeunload") {
        const action = beforeUnloadAction || "accept";
        beforeUnloadAction = null;
        return { action, unexpected: false };
      }

      if (expected && dialog.type === expected.type && dialog.message === expected.message) {
        const response = { action: expected.action, promptText: expected.promptText, unexpected: false };
        expected = null;
        return response;
      }

      unexpected.push(`Unexpected native ${dialog.type}: ${dialog.message}`);
      return { action: "dismiss", unexpected: true };
    },

    hasPendingExpectation() {
      return expected !== null;
    },

    failures() {
      const failures = unexpected.slice();
      if (expected) failures.push(`Expected native ${expected.type} was not shown: ${expected.message}`);
      return failures;
    },
  };
}

module.exports = { createNativeDialogPolicy };
