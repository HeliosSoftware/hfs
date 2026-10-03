const test = require("node:test");
const assert = require("node:assert/strict");

const editorAdd = require("../../assets/editor-add.js");

// #1239: `matches`, `parentPath`, and `leafName` are the add-picker's own
// pure rules, exported so they can be exercised without a DOM.
// `capturePickers` and `restorePickers` need real elements, so they stay
// covered by the existing Playwright specs over the three hosts instead.

test("an empty needle matches everything", () => {
  assert.equal(editorAdd.matches("birthDate", ""), true);
  assert.equal(editorAdd.matches("", ""), true);
});

test("a matching substring is found regardless of position", () => {
  assert.equal(editorAdd.matches("birthDate", "date"), true);
  assert.equal(editorAdd.matches("birthDate", "birth"), true);
  assert.equal(editorAdd.matches("birthDate", "hDa"), true);
});

test("the match is case-insensitive", () => {
  assert.equal(editorAdd.matches("birthDate", "DATE"), true);
  assert.equal(editorAdd.matches("BirthDate", "date"), true);
});

test("no match returns false", () => {
  assert.equal(editorAdd.matches("birthDate", "xyz"), false);
});

test("parentPath drops a trailing index and the last path segment", () => {
  assert.equal(editorAdd.parentPath("birthDate"), "");
  assert.equal(editorAdd.parentPath("name[1]"), "");
  assert.equal(editorAdd.parentPath("name[0].family"), "name[0]");
  assert.equal(editorAdd.parentPath("name[0].given[2]"), "name[0]");
  assert.equal(editorAdd.parentPath("extension[0]"), "");
  assert.equal(editorAdd.parentPath(""), "");
  // The editor's own dotted spelling (`crates/ui/src/editor.rs`).
  assert.equal(editorAdd.parentPath("name.1"), "");
  assert.equal(editorAdd.parentPath("extension.0"), "");
  assert.equal(editorAdd.parentPath("name.0.family"), "name.0");
  assert.equal(editorAdd.parentPath("name.0.given.2"), "name.0");
  assert.equal(editorAdd.parentPath("name.0.extension.0"), "name.0");
});

test("leafName drops a trailing index and keeps the last path segment", () => {
  assert.equal(editorAdd.leafName("birthDate"), "birthDate");
  assert.equal(editorAdd.leafName("name[1]"), "name");
  assert.equal(editorAdd.leafName("name[0].family"), "family");
  assert.equal(editorAdd.leafName("valueString"), "valueString");
  assert.equal(editorAdd.leafName("extension[0]"), "extension");
  assert.equal(editorAdd.leafName("name.1"), "name");
  assert.equal(editorAdd.leafName("extension.0"), "extension");
  assert.equal(editorAdd.leafName("name.0.family"), "family");
  assert.equal(editorAdd.leafName("name.0.given.2"), "given");
});

// Small element doubles exercise the helper contract without introducing a
// DOM library. Browser specs own real focus order, clipping and live regions.
function control(path) {
  return {
    dataset: { set: path }, disabled: false, attributes: {},
    closest() { return null; },
    focus(options) { this.focusOptions = options; },
    select() { this.selected = true; },
    setAttribute(key, value) { this.attributes[key] = value; },
    removeAttribute(key) { delete this.attributes[key]; },
    getBoundingClientRect() { return { top: 160, bottom: 188 }; },
  };
}

function fixture({ primitive = true, path = "base.0", doc = '{"base":[""]}' } = {}) {
  const input = control(path);
  const child = control(path + ".value");
  const head = {
    getBoundingClientRect() { return { top: 160, bottom: 188 }; },
    insertAdjacentElement(_position, note) { note.placed = true; },
  };
  const row = {
    dataset: { path },
    querySelectorAll() { return primitive ? [input] : [child]; },
    querySelector(selector) { return selector === ".editor-row__head" ? head : null; },
    focus(options) { this.focusOptions = options; },
    getBoundingClientRect() { return { top: 160, bottom: 228 }; },
  };
  const undo = control("");
  const note = { hidden: true, querySelector() { return undo; } };
  undo.closest = () => note;
  const status = {
    textContent: "", dataset: {
      msgAdded: "added", msgPending: "Undo pending", msgUnavailable: "Undo unavailable", msgFailed: "Retry the changed field",
    },
  };
  const tree = {
    scrollTop: 0, clientTop: 0, clientHeight: 300, style: {},
    getBoundingClientRect() { return { top: 100, bottom: 400 }; },
  };
  const container = {
    querySelectorAll(selector) { return selector === "[data-set]" ? [input] : [row]; },
    querySelector(selector) {
      return {
        ".editor-tree": tree, "[data-add-status]": status,
        "[data-add-undo-note]": note, "[data-add-undo]": undo,
        "#editor-doc": { value: doc },
      }[selector] || null;
    },
    contains(element) { return [row, input, note, undo, status].includes(element); },
  };
  return { container, row, input, child, tree, note, undo, status };
}

const directOperation = { originPath: "base", pickerPath: null };

test("operation identity distinguishes a collection picker from its schema parent", () => {
  const row = { dataset: { path: "name.0.given" } };
  const box = { closest() { return row; } };
  const trigger = { closest(selector) { return selector.startsWith("details") ? box : row; } };
  assert.deepEqual(editorAdd.operationFrom(trigger), {
    originPath: "name.0.given", pickerPath: "name.0.given",
  });
  trigger.closest = (selector) => selector.startsWith("details") ? null : row;
  assert.deepEqual(editorAdd.operationFrom(trigger), { originPath: "name.0.given", pickerPath: null });
  assert.equal(editorAdd.parentPath("name.0.given.1"), "name.0");
});

test("created primitive gets exact focus and selection, without scrolling a visible input", () => {
  const f = fixture();
  assert.equal(editorAdd.revealCreated(f.container, "base.0", directOperation), true);
  assert.deepEqual(f.input.focusOptions, { preventScroll: true });
  assert.equal(f.input.selected, true);
  assert.equal(f.tree.scrollTop, 0);
  assert.equal(f.note.placed, true);
  assert.equal(f.note.hidden, false);
  assert.equal(f.undo.dataset.remove, "base.0");
});

test("complex creation focuses its row rather than a descendant primitive", () => {
  const f = fixture({ primitive: false, path: "identifier.0", doc: '{"identifier":[{}]}' });
  assert.equal(editorAdd.revealCreated(f.container, "identifier.0", { originPath: "", pickerPath: null }), true);
  assert.deepEqual(f.row.focusOptions, { preventScroll: true });
  assert.equal(f.child.focusOptions, undefined);
});

test("extension creation ignores its hidden child-choice selector and focuses the row", () => {
  const f = fixture({ primitive: false, path: "extension.0", doc: '{"extension":[{"url":"http://example.org/e"}]}' });
  const choice = control("");
  choice.dataset = { choose: "extension.0" };
  // The nearest Elements disclosure is open, but its enclosing picker is
  // closed. This is a child-add control, never an editable extension value.
  choice.closest = () => ({ open: true, closest() { return { open: false }; } });
  f.row.querySelectorAll = (selector) => selector.includes("data-choose") ? [f.child, choice] : [f.child];
  assert.equal(editorAdd.revealCreated(f.container, "extension.0", { originPath: "", pickerPath: null }), true);
  assert.deepEqual(f.row.focusOptions, { preventScroll: true });
  assert.equal(choice.focusOptions, undefined);
});

test("fitting complex reveal includes the newly placed Undo and child Add actions", () => {
  const f = fixture({ primitive: false, path: "contact.0", doc: '{"contact":[{}]}' });
  f.row.getBoundingClientRect = () => ({ top: 369, bottom: f.note.placed ? 472 : 420 });
  f.row.querySelector(".editor-row__head").getBoundingClientRect = () => ({ top: 369, bottom: 397 });
  editorAdd.revealCreated(f.container, "contact.0", { originPath: "", pickerPath: null });
  assert.equal(f.note.placed, true);
  assert.equal(f.tree.scrollTop, 72); // the heading alone was already visible
});

test("oversized complex row reveals its heading rather than the full subtree", () => {
  const f = fixture({ primitive: false, path: "contact.0", doc: '{"contact":[{}]}' });
  f.row.getBoundingClientRect = () => ({ top: 369, bottom: 800 });
  f.row.querySelector(".editor-row__head").getBoundingClientRect = () => ({ top: 369, bottom: 397 });
  editorAdd.revealCreated(f.container, "contact.0", { originPath: "", pickerPath: null });
  assert.equal(f.tree.scrollTop, 0);
});

test("missing or empty created path does not fabricate Undo or report a reveal", () => {
  const f = fixture();
  assert.equal(editorAdd.revealCreated(f.container, "", directOperation), false);
  assert.equal(editorAdd.revealCreated(f.container, "base", directOperation), false);
  assert.equal(f.note.hidden, true);
  assert.equal(f.input.focusOptions, undefined);
});

test("pending blur mutation disables Undo synchronously and blocks stale removal", () => {
  const f = fixture();
  editorAdd.revealCreated(f.container, "base.0", directOperation);
  const finish = editorAdd.beginRequest(f.container);
  assert.equal(f.undo.disabled, true);
  assert.equal(f.undo.attributes["aria-busy"], "true");
  assert.equal(f.undo.title, "Undo pending");
  assert.equal(editorAdd.undoOperation(f.container, f.undo, '{"base":[""]}'), null);
  // A network failure without a swap leaves the original operation usable.
  finish();
  assert.equal(f.undo.disabled, false);
  assert.equal(f.undo.attributes["aria-busy"], undefined);
  assert.deepEqual(editorAdd.undoOperation(f.container, f.undo, '{ "base": [""] }'), {
    undo: true, originPath: "base", createdPath: "base.0",
  });
});

test("overlapping request completion cannot enable Undo while another request is pending", () => {
  const f = fixture();
  editorAdd.revealCreated(f.container, "base.0", directOperation);
  const first = editorAdd.beginRequest(f.container);
  const second = editorAdd.beginRequest(f.container);
  first();
  first(); // completion is idempotent
  assert.equal(f.undo.disabled, true);
  second();
  assert.equal(f.undo.disabled, false);
});

test("raw document changed before debounced refresh makes old indexed Undo unavailable", () => {
  const f = fixture({ doc: '{"base":["Patient"]}' });
  editorAdd.revealCreated(f.container, "base.0", directOperation);
  assert.equal(editorAdd.undoOperation(f.container, f.undo, '{"base":["Observation"]}'), null);
  assert.equal(f.note.hidden, true);
});

test("reveal only scrolls the tree enough for an offscreen control", () => {
  const f = fixture();
  const target = { getBoundingClientRect() { return { top: 420, bottom: 448 }; } };
  editorAdd.revealInTree(f.tree, target);
  assert.equal(f.tree.scrollTop, 48);
});

test("offscreen last input bounds a tree extending below the window before reveal", () => {
  const previous = global.window;
  global.window = { innerHeight: 380 };
  try {
    const f = fixture();
    const target = { getBoundingClientRect() { return { top: 390, bottom: 418 }; } };
    editorAdd.revealInTree(f.tree, target);
    assert.equal(f.tree.style.maxHeight, "280px");
    assert.equal(f.tree.scrollTop, 38);
    f.tree.style.maxHeight = "";
    editorAdd.revealInTree(f.tree, f.input);
    assert.equal(f.tree.style.maxHeight, ""); // visible input does not resize
  } finally {
    if (previous === undefined) delete global.window;
    else global.window = previous;
  }
});

test("Undo focuses the surviving group Add or root Add after last-item removal", () => {
  const collectionAdd = control("");
  const rootSummary = control("");
  const collection = {
    dataset: { path: "base" },
    querySelector(selector) { return selector === "[data-collection-add]" ? collectionAdd : null; },
  };
  const root = {
    dataset: { path: "" },
    querySelector() { return { querySelector() { return rootSummary; } }; },
  };
  let rows = [root, collection];
  const container = {
    querySelectorAll() { return rows; },
    querySelector() { return null; },
  };
  const operation = { undo: true, originPath: "base", createdPath: "base.1" };
  assert.equal(editorAdd.restoreUndoFocus(container, operation), true);
  assert.deepEqual(collectionAdd.focusOptions, { preventScroll: true });
  rows = [root];
  assert.equal(editorAdd.restoreUndoFocus(container, operation), true);
  assert.deepEqual(rootSummary.focusOptions, { preventScroll: true });
});

test("restore closes only the actual scoped picker and retains unrelated filter state", () => {
  function pickerRow(path) {
    const filter = { value: "", dispatchEvent() { this.filtered = true; }, focus() { this.focused = true; } };
    const box = {
      open: true,
      querySelector(selector) { return selector === ".editor-add__filter" ? filter : null; },
      setAttribute() { this.open = true; },
      removeAttribute() { this.open = false; },
    };
    return { dataset: { path }, querySelector() { return box; }, box, filter };
  }
  const root = pickerRow("");
  const collection = pickerRow("base");
  const container = { querySelectorAll() { return [root, collection]; } };
  const saved = [
    { path: "", filter: "birth", focusFilter: false },
    { path: "base", filter: "slice", focusFilter: true },
  ];
  editorAdd.restorePickers(container, saved, "base.1", { originPath: "base", pickerPath: "base" });
  assert.equal(collection.box.open, false);
  assert.equal(collection.filter.focused, undefined);
  assert.equal(root.box.open, true);
  assert.equal(root.filter.value, "birth");
  assert.equal(root.filter.filtered, true);

  // A direct header button has no picker owner, even though its created
  // path still has the root as its schema mutation parent.
  editorAdd.restorePickers(container, saved, "base.2", directOperation);
  assert.equal(root.box.open, true);
  assert.equal(collection.box.open, true);
});

test("mounted live status announces consecutive additions of the same field", async () => {
  const f = fixture();
  editorAdd.revealCreated(f.container, "base.0", directOperation);
  assert.equal(f.status.textContent, "");
  await new Promise((resolve) => setTimeout(resolve, 70));
  assert.equal(f.status.textContent, "base added");
  editorAdd.revealCreated(f.container, "base.0", directOperation);
  assert.equal(f.status.textContent, "");
  await new Promise((resolve) => setTimeout(resolve, 70));
  assert.equal(f.status.textContent, "base added");
});

test("dependent mutations read the document after their prerequisite swap", async () => {
  const f = fixture();
  let document = [""];
  let release;
  const held = new Promise(resolve => { release = resolve; });
  const seen = [];
  const first = editorAdd.queueMutation(f.container, async () => {
    seen.push({ op: "set", doc: document.slice() });
    await held;
    document = ["Patient"];
  });
  const second = editorAdd.queueMutation(f.container, async () => {
    seen.push({ op: "add", doc: document.slice() });
    document.push("");
  });
  assert.equal(f.undo.disabled, true); // reserved before either fetch starts
  await Promise.resolve();
  assert.deepEqual(seen, [{ op: "set", doc: [""] }]);
  release();
  await Promise.all([first, second]);
  assert.deepEqual(seen[1], { op: "add", doc: ["Patient"] });
  assert.deepEqual(document, ["Patient", ""]);
  assert.equal(f.undo.disabled, false);
});

test("a failed prerequisite cancels dependent mutations and a later explicit retry can proceed", async () => {
  const f = fixture();
  f.input.value = "Patient";
  let appends = 0;
  const set = editorAdd.queueMutation(f.container, () => Promise.reject(new Error("network")), () => {
    editorAdd.failedMutation(f.container, "set", { path: "base.0" });
  });
  const add = editorAdd.queueMutation(f.container, () => { appends++; });
  const results = await Promise.allSettled([set, add]);
  assert.deepEqual(results.map(result => result.status), ["rejected", "rejected"]);
  assert.equal(appends, 0);
  assert.equal(f.input.value, "Patient");
  assert.deepEqual(f.input.focusOptions, { preventScroll: true });
  assert.equal(f.input.attributes["aria-invalid"], "true");
  assert.equal(f.input.title, "Retry the changed field");
  await editorAdd.queueMutation(f.container, () => { appends++; });
  assert.equal(appends, 1);
});

test("queuing a mutation invalidates earlier background refresh versions immediately", async () => {
  const f = fixture();
  const rawVersion = editorAdd.documentVersion(f.container);
  const mutation = editorAdd.queueMutation(f.container, () => {});
  assert.notEqual(editorAdd.documentVersion(f.container), rawVersion);
  await mutation;
  const installedVersion = editorAdd.documentVersion(f.container);
  editorAdd.invalidateRefresh(f.container); // external document installation
  assert.notEqual(editorAdd.documentVersion(f.container), installedVersion);
});


test("structural reservation blocks stale paths, preserves schema disabled state and unlocks before focus", async () => {
  const editable = control("identifier.1.value");
  const schemaDisabled = control("fixed"); schemaDisabled.disabled = true;
  let controls = [editable, schemaDisabled];
  const attrs = {};
  const container = {
    querySelectorAll() { return controls; }, querySelector() { return null; },
    setAttribute(key, value) { attrs[key] = value; }, removeAttribute(key) { delete attrs[key]; },
  };
  let release;
  const held = new Promise(resolve => { release = resolve; });
  let staleRequests = 0;
  const removal = editorAdd.queueMutation(container, async () => {
    await held;
    const fresh = control("identifier.0.value");
    controls = [fresh];
    editorAdd.projectionSwapped(container, "remove");
    assert.equal(fresh.disabled, false);
    assert.equal(editorAdd.projectionBusy(container), false);
  }, null, "remove");
  assert.equal(editable.disabled, true);
  assert.equal(attrs["aria-busy"], "true");
  await editorAdd.queueMutation(container, () => { staleRequests++; }, null, "remove");
  await editorAdd.queueMutation(container, () => { staleRequests++; }, null, "set");
  release(); await removal;
  assert.equal(staleRequests, 0);
  assert.equal(editable.disabled, false);
  assert.equal(schemaDisabled.disabled, true);
  assert.equal(attrs["aria-busy"], undefined);
});

test("intermediate set swaps stay blocked, and failed prerequisites release the retained projection before retry feedback", async () => {
  let controls = [control("base.0")];
  const container = { querySelectorAll() { return controls; }, querySelector() { return null; } };
  let reject;
  const held = new Promise((_resolve, fail) => { reject = fail; });
  let restored = false;
  const set = editorAdd.queueMutation(container, () => held, () => {
    assert.equal(editorAdd.projectionBusy(container), false);
    assert.equal(controls[0].disabled, false);
    restored = true;
  }, "set");
  const add = editorAdd.queueMutation(container, () => assert.fail("failed prerequisite must cancel Add"), null, "add");
  controls = [control("base.0")];
  editorAdd.projectionSwapped(container, "set");
  assert.equal(controls[0].disabled, true);
  await Promise.resolve(); reject(new Error("failed"));
  await Promise.allSettled([set, add]);
  assert.equal(restored, true);
  assert.equal(editorAdd.projectionBusy(container), false);
  assert.equal(controls[0].disabled, false);
});


test("a dirty primitive holds pointer focus until click flushes it before the action", () => {
  const previous = global.document;
  const order = [];
  const input = { value: "Patient", defaultValue: "", matches() { return true; }, blur() { order.push("set"); } };
  const action = { disabled: false };
  const container = { contains(node) { return node === input; } };
  const event = { button: 0, target: { closest() { return action; } }, preventDefault() { order.push("prevent pointer blur"); } };
  global.document = { activeElement: input };
  try {
    editorAdd.holdDirtyPointer(container, event);
    assert.deepEqual(order, ["prevent pointer blur"]);
    editorAdd.flushDirtyPrimitive(container, event);
    order.push("add");
    assert.deepEqual(order, ["prevent pointer blur", "set", "add"]);
    order.length = 0;
    input.value = input.defaultValue;
    editorAdd.holdDirtyPointer(container, event);
    assert.deepEqual(order, []);
    action.disabled = true;
    editorAdd.flushDirtyPrimitive(container, event);
    assert.deepEqual(order, []);
    action.disabled = false; input.value = "Patient"; event.button = 2;
    editorAdd.holdDirtyPointer(container, event);
    assert.deepEqual(order, []);
    event.button = 0; event.ctrlKey = true;
    editorAdd.holdDirtyPointer(container, event);
    assert.deepEqual(order, []);
  } finally { global.document = previous; }
});
