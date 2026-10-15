const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

// #2049: executes the production sql-export-form.js against hand-built DOM
// stand-ins, the way delegated-lifecycle.test.cjs runs conformance-crud.js.
// The browser ring (e2e/tests/sql-export.spec.ts) covers real layout, focus
// and accessible-description computation; this ring pins the script's own
// decisions without a server or a browser.
//
// The stand-ins answer only the selectors the script asks for at load.
// Anything unmapped returns null / [] (never throws), so the script's guard
// branches run as they do in a real document that lacks that element.

const SOURCE = fs.readFileSync(path.resolve(__dirname, "../../assets/sql-export-form.js"), "utf8");

const FIELD_ID = "param:Library/q1:ward-error";

function element({ attrs = {}, select = {}, selectAll = {}, ...fields } = {}) {
  const attributes = new Map(Object.entries(attrs));
  const listeners = new Map();
  return {
    hidden: false,
    textContent: "",
    dataset: {},
    getAttribute(name) {
      return attributes.has(name) ? attributes.get(name) : null;
    },
    setAttribute(name, value) {
      attributes.set(name, String(value));
    },
    removeAttribute(name) {
      attributes.delete(name);
    },
    hasAttribute(name) {
      return attributes.has(name);
    },
    querySelector(selector) {
      return Object.hasOwn(select, selector) ? select[selector] : null;
    },
    querySelectorAll(selector) {
      return Object.hasOwn(selectAll, selector) ? selectAll[selector] : [];
    },
    addEventListener(type, listener) {
      const handlers = listeners.get(type) || [];
      handlers.push(listener);
      listeners.set(type, handlers);
    },
    dispatch(type, event = {}) {
      (listeners.get(type) || []).slice().forEach((listener) => listener(event));
    },
    ...fields,
  };
}

/* One parameterized SQL Query (`Library/q1`, one `ward` parameter with no
   default) in a subjects table, mounted by running the real script. With
   `serverError` the page is the server's re-render after a failed
   submission: the control already carries aria-invalid and aria-describedby,
   its error span is visible, and the values row is `data-open`. */
function mount({ serverError = false, describedBy } = {}) {
  const controlAttrs = {};
  if (serverError) {
    controlAttrs["aria-invalid"] = "true";
    controlAttrs["aria-describedby"] = FIELD_ID;
  }
  if (describedBy) controlAttrs["aria-describedby"] = describedBy;

  const mounted = {};
  const control = element({
    attrs: controlAttrs,
    value: "",
    focus() {
      mounted.focused = control;
    },
  });
  control.name = "param:Library/q1:ward";

  const errorSpan = element({
    hidden: !serverError,
    textContent: serverError ? "Server error." : "",
    id: FIELD_ID,
  });

  const field = element({
    dataset: { paramName: "ward" },
    select: { "input, select": control, ".field__hint--error": errorSpan },
  });

  const paramsRow = element({
    classList: { contains: (name) => name === "row--params" },
    selectAll: { ".field[data-param-name]": [field] },
    hasAttribute: (name) => name === "data-open" && serverError,
  });

  const toggle = element({ attrs: { "aria-expanded": "true" }, hidden: true });
  const summary = element({ hidden: true, appendChild() {} });
  const box = element({ checked: serverError });

  const row = element({
    hidden: false,
    dataset: { kind: "sql-query", name: "ward_query" },
    nextElementSibling: paramsRow,
    select: {
      ".row-toggle": toggle,
      ".param-summary": summary,
      'input[name="subject"]': box,
    },
  });

  const countHint = element({
    dataset: {
      msgCount: "{selected} of {total} selected",
      msgMissingOne: "{count} value missing",
      msgMissingOther: "{count} values missing",
      msgParamRequired: "This value is required.",
      msgParamRequiredChip: "required",
    },
  });

  const tableSelect = { "[data-msg-count]": countHint };
  if (serverError) tableSelect[".row--params[data-open] [aria-invalid='true']"] = control;
  const table = element({
    select: tableSelect,
    selectAll: { "tbody tr[data-kind]": [row] },
  });

  const form = element({ select: { ".table-card": table } });

  const document = {
    querySelector: (selector) => (selector === "form.bulk-export-form" ? form : null),
    getElementById: (id) => (id === FIELD_ID ? errorSpan : null),
    createElement: () => element(),
  };
  const window = { HfsNumber: { format: String } };
  vm.runInContext(SOURCE, vm.createContext({ document, window }), {
    filename: "sql-export-form.js",
  });

  const submit = () => {
    const event = {
      prevented: false,
      preventDefault() {
        this.prevented = true;
      },
    };
    form.dispatch("submit", event);
    return event;
  };

  return Object.assign(mounted, { control, errorSpan, paramsRow, box, summary, submit });
}

/* Ticks the query the way a click on its checkbox does. */
function check(page) {
  page.box.checked = true;
  page.box.dispatch("change");
}

/* Types into the field the way a keystroke does. */
function type(page, value) {
  page.control.value = value;
  page.control.dispatch("input");
}

test("a blocked submit shows the required message under the empty field, marks it, describes it and focuses it (#2049)", () => {
  const page = mount();
  check(page);

  const event = page.submit();

  assert.equal(event.prevented, true);
  assert.equal(page.paramsRow.hidden, false);
  assert.equal(page.control.getAttribute("aria-invalid"), "true");
  assert.equal(page.errorSpan.hidden, false);
  assert.equal(page.errorSpan.textContent, "This value is required.");
  assert.equal(page.control.getAttribute("aria-describedby"), FIELD_ID);
  assert.equal(page.focused, page.control);
});

test("typing a value hides the message and drops the description again", () => {
  const page = mount();
  check(page);
  page.submit();

  type(page, "W1");

  assert.equal(page.errorSpan.hidden, true);
  assert.equal(page.control.hasAttribute("aria-invalid"), false);
  assert.equal(page.control.hasAttribute("aria-describedby"), false);
  assert.equal(page.submit().prevented, false);
});

test("an unchecked query never blocks the submit or shows a message", () => {
  const page = mount();

  const event = page.submit();

  assert.equal(event.prevented, false);
  assert.equal(page.errorSpan.hidden, true);
  assert.equal(page.control.hasAttribute("aria-invalid"), false);
  assert.equal(page.control.hasAttribute("aria-describedby"), false);
  assert.equal(page.focused, undefined);
});

test("a server-rendered error clears on input without leaving a stale description", () => {
  const page = mount({ serverError: true });

  // The existing `data-open` path: the flagged field takes focus on load,
  // with the server's own message still showing.
  assert.equal(page.focused, page.control);
  assert.equal(page.errorSpan.hidden, false);
  assert.equal(page.errorSpan.textContent, "Server error.");
  assert.equal(page.control.getAttribute("aria-invalid"), "true");
  assert.equal(page.control.getAttribute("aria-describedby"), FIELD_ID);

  type(page, "W1");

  assert.equal(page.errorSpan.hidden, true);
  assert.equal(page.control.hasAttribute("aria-invalid"), false);
  assert.equal(page.control.hasAttribute("aria-describedby"), false);
});

test("other aria-describedby ids on the control are kept", () => {
  const page = mount({ describedBy: "other-hint" });
  check(page);

  page.submit();
  assert.equal(page.control.getAttribute("aria-describedby"), `other-hint ${FIELD_ID}`);

  type(page, "W1");
  assert.equal(page.control.getAttribute("aria-describedby"), "other-hint");
});
