const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

// Execute the production scripts, retaining the document while swapping its
// controls. Browser tests cover actual boost/history/keyboard behavior.
function documentDouble() {
  const listeners = new Map();
  return {
    querySelector() { return { content: "tenant-lifecycle" }; },
    addEventListener(type, listener) {
      const handlers = listeners.get(type) || [];
      handlers.push(listener);
      listeners.set(type, handlers);
    },
    dispatch(type, event) {
      (listeners.get(type) || []).slice().forEach((listener) => listener(event));
    },
  };
}

function load(context, name, times = 4) {
  const source = fs.readFileSync(path.resolve(__dirname, "../../assets", name), "utf8");
  for (let i = 0; i < times; i++) vm.runInContext(source, context, { filename: name });
}

const flush = () => new Promise(setImmediate);

test("conformance delete asks once after repeated loading and handles a replacement button", async () => {
  const document = documentDouble();
  const questions = [];
  const requests = [];
  const held = new WeakSet();
  let confirmed = false;
  let suspended = 0;
  const fetch = (url, options) => {
    requests.push({ url, options });
    return Promise.resolve({ ok: true });
  };
  const window = {
    fetch,
    HfsConfirm: { ask(message) { questions.push(message); return Promise.resolve(confirmed); } },
    hfsBusy: { during(buttons, work) {
      if (buttons.some((button) => held.has(button))) return null;
      buttons.forEach((button) => held.add(button));
      return work();
    } },
    HfsUnsaved: { suspend() { suspended++; } },
  };
  const context = vm.createContext({ document, window, fetch });
  const button = (id) => ({ dataset: {
    type: "CompartmentDefinition", id, confirm: `Delete ${id}?`, redirect: "/ui/compartments?refresh=1",
  } });
  const click = (btn) => document.dispatch("click", { target: { closest() { return btn; } } });

  load(context, "conformance-crud.js");
  click(button("first"));
  await flush();
  assert.deepEqual(questions, ["Delete first?"]);
  assert.deepEqual(requests, []);

  confirmed = true;
  load(context, "conformance-crud.js");
  click(button("replacement"));
  await flush();
  assert.deepEqual(questions, ["Delete first?", "Delete replacement?"]);
  assert.equal(requests.length, 1);
  assert.equal(requests[0].url, "/CompartmentDefinition/replacement");
  assert.equal(requests[0].options.method, "DELETE");
  assert.equal(requests[0].options.headers["X-Tenant-ID"], "tenant-lifecycle");
  assert.equal(suspended, 1);
  assert.equal(window.location, "/ui/compartments?refresh=1");
});

test("row delegation activates each replacement link once and preserves click exclusions", () => {
  const document = documentDouble();
  let selection = { isCollapsed: true };
  const window = { getSelection() { return selection; } };
  const context = vm.createContext({ document, window });
  const row = () => {
    let clicks = 0;
    const anchor = { click() { clicks++; } };
    const row = { querySelector() { return anchor; }, contains() { return true; } };
    const target = { closest(selector) { return selector.startsWith("table[") ? row : null; } };
    return { target, clicks: () => clicks };
  };
  const click = (target, props = {}) => {
    const event = { target, button: 0, defaultPrevented: false,
      preventDefault() { this.defaultPrevented = true; }, ...props };
    document.dispatch("click", event);
    return event;
  };
  load(context, "row-navigation.js");
  const first = row();
  click(first.target);
  assert.equal(first.clicks(), 1);

  load(context, "row-navigation.js");
  const replacement = row();
  click(replacement.target);
  assert.equal(replacement.clicks(), 1);
  for (const modifier of ["ctrlKey", "metaKey", "shiftKey", "altKey"]) {
    click(replacement.target, { [modifier]: true });
  }
  click(replacement.target, { button: 1 });
  click(replacement.target, { defaultPrevented: true });
  click({ closest(selector) { return selector.startsWith("table[") ? {} : {}; } });
  assert.equal(replacement.clicks(), 1);

  selection = { isCollapsed: false, anchorNode: {} };
  assert.equal(click(replacement.target).defaultPrevented, true);
  assert.equal(replacement.clicks(), 1);
});

function jsonView() {
  const classes = () => {
    const values = new Set();
    return {
      contains(value) { return values.has(value); },
      toggle(value, enabled) { if (enabled) values.add(value); else values.delete(value); },
    };
  };
  const view = { querySelectorAll() { return [root, child]; } };
  const arrow = { dataset: { fold: "root" }, attributes: {},
    closest() { return view; }, setAttribute(name, value) { this.attributes[name] = value; } };
  const root = { dataset: { foldId: "root", parents: "" }, classList: classes(),
    querySelector() { return arrow; } };
  const childArrow = { setAttribute() {} };
  const child = { dataset: { foldId: "child", parents: "root" }, classList: classes(),
    querySelector() { return childArrow; } };
  return { arrow, root, child, target: { closest(selector) { return selector === "[data-fold]" ? arrow : null; } },
    control(collapse) { return { closest(selector) { return selector === "[data-json-fold]"
      ? { dataset: { jsonFold: collapse ? "all" : "none" }, closest() { return { querySelector() { return view; } }; } }
      : null; } }; } };
}

test("JSON folds toggle once after repeated loading and stay delegated to new fragments", () => {
  const document = documentDouble();
  const context = vm.createContext({ document });
  const click = (target) => document.dispatch("click", { target });
  load(context, "json-view.js");
  const first = jsonView();
  click(first.target);
  assert.equal(first.root.classList.contains("json-line--collapsed"), true);
  assert.equal(first.child.hidden, true);
  assert.equal(first.arrow.attributes["aria-expanded"], "false");
  click(first.target);
  assert.equal(first.child.hidden, false);

  load(context, "json-view.js");
  const replacement = jsonView();
  click(replacement.target);
  assert.equal(replacement.child.hidden, true);
  click(replacement.target);
  assert.equal(replacement.child.hidden, false);
  click(replacement.control(true));
  assert.equal(replacement.root.classList.contains("json-line--collapsed"), false);
  assert.equal(replacement.child.classList.contains("json-line--collapsed"), true);
  click(replacement.control(false));
  assert.equal(replacement.child.classList.contains("json-line--collapsed"), false);
});
