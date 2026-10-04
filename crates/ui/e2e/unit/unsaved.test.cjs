const test = require("node:test");
const assert = require("node:assert/strict");

const unsaved = require("../../assets/unsaved.js");

// #1240: the shared unsaved-changes tracker's comparison — trimmed values,
// JSON compared by canonical content rather than letter by letter (the same
// rule editor-pair.js already uses for its own JSON<->form sync). `serialize`
// needs a real DOM (`form.elements`), so it is exercised in Playwright
// instead; these tests only need `normalize`, plain strings, no window/
// document at all.

test("normalize trims leading and trailing whitespace", () => {
  assert.equal(unsaved.normalize("  a b  "), "a b");
  assert.equal(unsaved.normalize("  "), "");
});

test("normalize compares JSON by content", () => {
  const spaced = '{ "a": 1,\n "b": [1, 2] }';
  const compact = '{"a":1,"b":[1,2]}';
  assert.equal(unsaved.normalize(spaced), unsaved.normalize(compact));
});

test("normalize keeps invalid JSON as text", () => {
  assert.equal(unsaved.normalize('{"a":'), '{"a":');
});

test("normalize leaves non-JSON text intact", () => {
  assert.equal(unsaved.normalize("SELECT 1"), "SELECT 1");
});

test("pending snapshots preserve JSON equivalence, disabled primitive edits and undo", () => {
  const field = { dataset: { set: "name.0.family" }, tagName: "INPUT", value: "Ana", defaultValue: "Ana", disabled: true };
  const root = { querySelectorAll: () => [field] };
  const baseline = unsaved.withPending('{ "name": [{ "family": "Ana" }] }', root);
  assert.equal(unsaved.withPending('{"name":[{"family":"Ana"}]}', root), baseline);
  field.value = "Bea";
  assert.notEqual(unsaved.withPending('{"name":[{"family":"Ana"}]}', root), baseline);
  field.value = " Ana ";
  assert.notEqual(unsaved.withPending('{"name":[{"family":"Ana"}]}', root), baseline);
  field.value = "Ana";
  assert.equal(unsaved.withPending('{"name":[{"family":"Ana"}]}', root), baseline);
});

test("pending selects compare the loaded option and invalid JSON remains guarded text", () => {
  const select = {
    dataset: { set: "gender" }, tagName: "SELECT", value: "female",
    options: [{ value: "male", defaultSelected: true }, { value: "female", defaultSelected: false }],
  };
  const emptySelect = { dataset: { set: "status" }, tagName: "SELECT", value: "draft", options: [{ value: "draft" }] };
  const root = { querySelectorAll: () => [select, emptySelect] };
  assert.match(unsaved.withPending('{"gender":', root), /gender="female"/);
  assert.doesNotMatch(unsaved.withPending('{"gender":', root), /status=/);
  select.value = "male";
  assert.equal(unsaved.withPending('{"gender":', root), '{"gender":');
});

test("fresh guards see edits before rAF and acknowledge discard without losing loaded-baseline undo", async () => {
  const previousWindow = global.window;
  const previousDocument = global.document;
  const windowListeners = {};
  const scheduled = [];
  let accepted = false;
  let confirms = 0;
  global.window = {
    addEventListener: (name, listener) => { windowListeners[name] = listener; },
    requestAnimationFrame: (listener) => { scheduled.push(listener); return scheduled.length; },
    confirm: () => { confirms++; return accepted; },
  };
  global.document = { body: { dataset: { msgUnsavedDiscard: "Discard?" } }, addEventListener() {} };
  let value = '{"family":"Ana"}';
  const listeners = {};
  const root = { isConnected: true, addEventListener: (name, listener) => { listeners[name] = listener; } };
  try {
    const tracker = unsaved.track({ root, checkOnExit: true, read: () => value });
    value = '{"family":"Bea"}';
    listeners.input();
    assert.equal(tracker.isDirty(), false, "no animation frame has run");
    const unload = { preventDefault() { this.prevented = true; } };
    windowListeners.beforeunload(unload);
    assert.equal(unload.prevented, true);
    assert.equal(unsaved.isDirty(root), true);
    assert.equal(await unsaved.confirmDiscard(root), false);
    assert.equal(confirms, 1);
    accepted = true;
    assert.equal(await unsaved.confirmDiscard(root), true);
    assert.equal(confirms, 2);
    assert.equal(unsaved.isDirty(root), false);
    scheduled.forEach(listener => listener());
    assert.equal(await unsaved.confirmDiscard(root), true, "accepted snapshot must not prompt again");
    assert.equal(confirms, 2);
    const cleanUnload = { preventDefault() { this.prevented = true; } };
    windowListeners.beforeunload(cleanUnload);
    assert.equal(cleanUnload.prevented, undefined, "accepted discard cannot raise a second navigation prompt");
    value = '{"family":"Cara"}';
    assert.equal(unsaved.isDirty(root), true);
    value = '{ "family": "Ana" }';
    assert.equal(unsaved.isDirty(root), false, "undo still compares against the loaded document");
    value = '{"family":"Bea"}';
    assert.equal(unsaved.isDirty(root), true, "a new edit is not the old discarded state");
    tracker.reset();
    assert.equal(unsaved.isDirty(root), false);
    assert.equal(typeof listeners["hfs:editor-mutation"], "function");
    root.isConnected = false;
    value = null;
    assert.equal(unsaved.isDirty(root), false);
    assert.equal(await unsaved.confirmDiscard(root), true);
  } finally {
    root.isConnected = false;
    global.window = previousWindow;
    global.document = previousDocument;
  }
});

test("only semantic mutations make an unchanged snapshot pending", async () => {
  const editorAdd = require("../../assets/editor-add.js");
  const previousWindow = global.window;
  global.window = { HfsEditorAdd: editorAdd };
  const root = { querySelectorAll: () => [], querySelector: () => null };
  try {
    const baseline = unsaved.withPending('{"resourceType":"Patient"}', root);
    const finishBackground = editorAdd.beginRequest(root);
    assert.equal(unsaved.withPending('{ "resourceType": "Patient" }', root), baseline);
    let release;
    const mutation = editorAdd.queueMutation(root, () => new Promise(resolve => { release = resolve; }), null, "set");
    assert.notEqual(unsaved.withPending('{"resourceType":"Patient"}', root), baseline);
    await Promise.resolve();
    release();
    await mutation;
    assert.equal(unsaved.withPending('{"resourceType":"Patient"}', root), baseline);
    finishBackground();
  } finally { global.window = previousWindow; }
});


test("pending primitive strings preserve JSON-looking text, whitespace and newlines exactly", () => {
  const field = { dataset: { set: "valueString" }, tagName: "INPUT", value: '{"a":1}', defaultValue: '{"a":1}' };
  const root = { querySelectorAll: () => [field] };
  const baseline = unsaved.withPending('{ "resourceType": "Parameters" }', root);
  field.value = '{ "a": 1 }';
  const spaced = unsaved.withPending('{"resourceType":"Parameters"}', root);
  assert.notEqual(spaced, baseline);
  assert.ok(spaced.includes('valueString=' + JSON.stringify(field.value)));
  field.value += " ";
  assert.notEqual(unsaved.withPending('{"resourceType":"Parameters"}', root), spaced);
  field.value += "\n";
  assert.ok(unsaved.withPending('{"resourceType":"Parameters"}', root).includes('valueString=' + JSON.stringify(field.value)));
  field.value = field.defaultValue;
  assert.equal(unsaved.withPending('{"resourceType":"Parameters"}', root), baseline);
});

test("acknowledged pending strings distinguish later whitespace edits and still allow exact undo", async () => {
  const priorWindow = global.window, priorDocument = global.document;
  let prompts = 0;
  global.window = { addEventListener() {}, requestAnimationFrame() {}, confirm() { prompts++; return true; } };
  global.document = { body: { dataset: { msgUnsavedDiscard: "Discard?" } }, addEventListener() {} };
  const field = { dataset: { set: "name.0.family" }, tagName: "INPUT", value: "Ana", defaultValue: "Ana" };
  const root = { isConnected: true, addEventListener() {}, querySelectorAll: () => [field] };
  try {
    const tracker = unsaved.track({ root, checkOnExit: true, read: () => unsaved.withPending('{"name":[{"family":"Ana"}]}', root) });
    field.value = " Ana ";
    assert.equal(await unsaved.confirmDiscard(root), true);
    assert.equal(prompts, 1);
    assert.equal(unsaved.isDirty(root), false);
    field.value = "  Ana ";
    assert.equal(unsaved.isDirty(root), true);
    assert.equal(await unsaved.confirmDiscard(root), true);
    assert.equal(prompts, 2);
    field.value = "Ana";
    assert.equal(tracker.check(), false);
  } finally { root.isConnected = false; global.window = priorWindow; global.document = priorDocument; }
});

test("default trackers keep untouched bootstrap changes clean until their lifecycle checks", async () => {
  const priorWindow = global.window, priorDocument = global.document;
  let prompts = 0;
  global.window = { addEventListener() {}, requestAnimationFrame() {}, confirm() { prompts++; return false; } };
  global.document = { body: { dataset: { msgUnsavedDiscard: "Discard?" } }, addEventListener() {} };
  let value = "";
  const root = { isConnected: true, addEventListener() {} };
  try {
    const tracker = unsaved.track({ root, read: () => value });
    value = "{}";
    assert.equal(unsaved.isDirty(root), false, "a visible loading placeholder is not an authored edit");
    assert.equal(await unsaved.confirmDiscard(root), true);
    assert.equal(prompts, 0, "untouched failed loading keeps the cached clean state");
    value = '{"resourceType":"Patient"}';
    tracker.reset();
    value = '{"resourceType":"Patient","active":true}';
    tracker.check();
    assert.equal(unsaved.isDirty(root), true, "event-driven edits retain the original guard");
    assert.equal(await unsaved.confirmDiscard(root), false);
    assert.equal(prompts, 1);
  } finally { root.isConnected = false; global.window = priorWindow; global.document = priorDocument; }
});
