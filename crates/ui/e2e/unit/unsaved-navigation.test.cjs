const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const source = fs.readFileSync(path.resolve(__dirname, '../../assets/unsaved.js'), 'utf8');
const flush = () => new Promise(setImmediate);

// Dispatch the production listeners in DOM capture/bubble order. The actual
// navigation/validation and dialog accessibility are also covered in browsers.
function fixture({ busy = false } = {}) {
  const questions = [];
  const requests = [];
  class Element {
    constructor(tag, attrs = {}) {
      this.tagName = tag;
      this.attrs = attrs;
      this.isConnected = true;
      this.listeners = [];
    }
    getAttribute(name) { return Object.hasOwn(this.attrs, name) ? this.attrs[name] : null; }
    setAttribute(name, value) { this.attrs[name] = value; }
    removeAttribute(name) { delete this.attrs[name]; }
    addEventListener(type, fn, capture = false) { this.listeners.push({ type, fn, capture }); }
    contains(other) { return other === this || other.parent === this; }
    closest(selector) { return selector === 'a[href]' && this.tagName === 'A' ? this : null; }
    focus() { document.activeElement = this; }
    click() { dispatch('click', { target: this, button: 0 }); }
  }
  class Form extends Element {
    constructor(attrs = {}) { super('FORM', attrs); this.controls = []; this.valid = true; }
    get elements() { return this.controls; }
    get method() { return this.attrs.method || 'get'; }
    get target() { return this.attrs.target || ''; }
    requestSubmit(submitter) {
      requests.push(submitter);
      if (this.valid || this.attrs.novalidate !== undefined || submitter?.attrs.formnovalidate !== undefined) {
        dispatch('submit', { target: this, submitter });
      }
    }
  }
  const body = new Element('BODY');
  body.contains = () => true;
  body.dataset = { msgUnsavedLeave: 'Discard your changes and leave?', msgUnsavedLeaveAction: 'Discard and leave' };
  const document = new Element('DOCUMENT');
  document.body = body;
  document.baseURI = 'http://example.test/ui/editor?mode=form';
  document.querySelector = () => null;
  const window = new Element('WINDOW');
  window.location = { href: document.baseURI, origin: 'http://example.test' };
  window.requestAnimationFrame = () => 1;
  window.HfsConfirm = { ask(message, options) {
    let resolve;
    const promise = new Promise(r => { resolve = r; });
    questions.push({ message, options, answer: resolve });
    return promise;
  } };
  const context = vm.createContext({ window, document, HTMLFormElement: Form, URL, Promise, setTimeout });
  if (busy) vm.runInContext(fs.readFileSync(path.resolve(__dirname, '../../assets/busy.js'), 'utf8'), context);
  vm.runInContext(source, context);
  function dispatch(type, properties = {}) {
    const event = { type, defaultPrevented: false, ...properties,
      preventDefault() { this.defaultPrevented = true; } };
    for (const capture of [true, false]) {
      for (const handler of document.listeners.filter(l => l.type === type && Boolean(l.capture) === capture)) handler.fn(event);
    }
    return event;
  }
  const root = new Element('MAIN');
  let value = '{"active":false}';
  const tracker = window.HfsUnsaved.track({ root, read: () => value, checkOnExit: true });
  function unload() {
    const event = { defaultPrevented: false, preventDefault() { this.defaultPrevented = true; } };
    window.listeners.find(l => l.type === 'beforeunload').fn(event);
    return event.defaultPrevented;
  }
  return { window, document, root, tracker, questions, requests, dispatch, unload,
    set: next => { value = next; }, dirty() { value = '{"active":true}'; },
    link: (attrs = {}) => new Element('A', { href: '/ui/resources?type=Patient#selected', ...attrs }),
    form: (attrs = {}) => new Form(attrs),
    button(form, attrs = {}) { const button = new Element('BUTTON', attrs); button.form = form; button.name = 'filter'; button.value = '1'; return button; },
    reloadScript() { vm.runInContext(source, context); },
    pageshow() { window.listeners.find(l => l.type === 'pageshow').fn({}); },
  };
}

function click(f, link, props = {}) { return f.dispatch('click', { target: link, button: 0, ...props }); }
function submit(f, form, button) { return f.dispatch('submit', { target: form, submitter: button }); }
function htmx(f, elt, props = {}) {
  return f.dispatch('htmx:confirm', { detail: { verb: 'get', elt, target: f.document.body,
    issueRequest(skipConfirm) { f.requests.push(skipConfirm); }, ...props } });
}


test('clean internal navigation remains direct and repeated script execution adds no listener', () => {
  const f = fixture();
  const count = f.document.listeners.length;
  f.reloadScript();
  assert.equal(f.document.listeners.length, count);
  assert.equal(click(f, f.link()).defaultPrevented, false);
  assert.equal(f.questions.length, 0);
});

test('Cancel preserves dirty state and undo still compares against the loaded baseline', async () => {
  const f = fixture(); f.dirty();
  assert.equal(click(f, f.link()).defaultPrevented, true);
  assert.equal(f.questions[0].options.confirmLabel, 'Discard and leave');
  f.questions[0].answer(false); await flush();
  assert.equal(f.window.HfsUnsaved.isDirty(), true);
  assert.equal(f.unload(), true);
  f.set('{ "active": false }');
  assert.equal(f.window.HfsUnsaved.isDirty(), false);
  assert.equal(f.unload(), false);
});

test('approved link activates once, consumes native permission, and keeps the draft dirty', async () => {
  const f = fixture(); f.dirty(); const link = f.link();
  let activations = 0;
  f.document.addEventListener('click', event => { if (event.target === link && !event.defaultPrevented) activations++; });
  click(f, link); click(f, f.link());
  assert.equal(f.questions.length, 1, 'concurrent actions do not stack questions');
  f.questions[0].answer(true); await flush();
  assert.equal(activations, 1);
  assert.equal(f.questions.length, 1);
  assert.equal(f.window.HfsUnsaved.isDirty(), true);
  assert.equal(f.unload(), false, 'the approved action does not also show native beforeunload');
  assert.equal(f.unload(), true, '204 or another retained document remains protected');
});

test('later cancellation of approved click revokes its native permission', async () => {
  const f = fixture(); f.dirty();
  f.document.addEventListener('click', event => event.preventDefault());
  click(f, f.link()); f.questions[0].answer(true); await flush();
  assert.equal(f.unload(), true);
});

test('disconnected or changed destination cannot consume an old authorization', async () => {
  for (const change of [link => { link.isConnected = false; }, link => { link.attrs.href = '/ui/history'; }]) {
    const f = fixture(); f.dirty(); const link = f.link();
    click(f, link); change(link); f.questions[0].answer(true); await flush();
    assert.equal(f.unload(), true);
    assert.equal(f.questions.length, 1);
  }
});

test('a genuinely changed draft asks again before resuming', async () => {
  const f = fixture(); f.dirty(); const link = f.link();
  click(f, link); f.set('{"active":true,"gender":"female"}');
  f.questions[0].answer(true); await flush();
  assert.equal(f.questions.length, 2);
  assert.equal(f.unload(), true);
  f.questions[1].answer(true); await flush();
  assert.equal(f.unload(), false);
});

test('settling a pending primitive into JSON does not ask twice for the same edit', async () => {
  const f = fixture();
  f.set('{"active":false}\n--pending--\nactive="true"\n--mutation--\n');
  click(f, f.link());
  f.set('{ "active": true }');
  f.dispatch('input', { target: { parent: f.root }, isTrusted: false });
  f.questions[0].answer(true); await flush();
  assert.equal(f.questions.length, 1);
  assert.equal(f.unload(), false);
  assert.equal(f.unload(), true);
});

test('a real edit after asking about pending primitives revokes the earlier answer', async () => {
  const f = fixture(); f.set('{"active":false}\n--pending--\nactive="true"');
  click(f, f.link()); f.set('{"active":true,"gender":"male"}');
  f.dispatch('input', { target: { parent: f.root }, isTrusted: true });
  f.questions[0].answer(true); await flush();
  assert.equal(f.questions.length, 2);
  assert.equal(f.unload(), true);
});

test('fragment, download, other-target, modified and external links retain their normal behavior', () => {
  const f = fixture(); f.dirty();
  const excluded = [ { href: '#tab' }, { href: '/ui/editor?mode=form#tab' }, { href: '#' },
    { download: '' }, { target: '_blank' }, { target: 'report' }, { href: 'mailto:alice@example.test' },
    { href: 'https://elsewhere.test/ui' }, { href: 'javascript:void(0)' } ];
  for (const attrs of excluded) assert.equal(click(f, f.link(attrs)).defaultPrevented, false, JSON.stringify(attrs));
  for (const modifier of ['ctrlKey', 'metaKey', 'shiftKey', 'altKey']) assert.equal(click(f, f.link(), { [modifier]: true }).defaultPrevented, false);
  assert.equal(click(f, f.link(), { button: 1 }).defaultPrevented, false);
  assert.equal(f.questions.length, 0);
  assert.equal(f.unload(), true);
});

test('the same URL without a fragment is a guarded reload, and base target is respected', () => {
  const f = fixture(); f.dirty();
  assert.equal(click(f, f.link({ href: f.window.location.href })).defaultPrevented, true);
  const g = fixture(); g.dirty();
  g.document.querySelector = () => ({ getAttribute: () => '_blank' });
  assert.equal(click(g, g.link()).defaultPrevented, false);
  assert.equal(click(g, g.link({ target: '_self' })).defaultPrevented, true);
});

test('missing translated navigation copy leaves native beforeunload active', () => {
  const f = fixture(); f.dirty(); delete f.document.body.dataset.msgUnsavedLeaveAction;
  assert.equal(click(f, f.link()).defaultPrevented, false);
  assert.equal(f.questions.length, 0);
  assert.equal(f.unload(), true);
});

test('missing shared modal falls back to exactly one native confirmation', async () => {
  const f = fixture(); f.dirty(); delete f.window.HfsConfirm;
  let asked = 0; f.window.confirm = () => { asked++; return true; };
  click(f, f.link()); await flush();
  assert.equal(asked, 1);
  assert.equal(f.unload(), false);
  assert.equal(f.unload(), true);
});

test('GET replay calls the form prototype once with its original submitter and overrides', async () => {
  const f = fixture(); f.dirty();
  const form = f.form({ method: 'post', action: '/ui/editor' });
  const button = f.button(form, { formmethod: 'get', formaction: '/ui/resources?mode=list', formtarget: '_self' });
  form.requestSubmit = 'a control shadows this method';
  assert.equal(submit(f, form, button).defaultPrevented, true);
  f.questions[0].answer(true); await flush();
  assert.deepEqual(f.requests, [button]);
  assert.equal(f.unload(), false);
  assert.equal(f.unload(), true);
});

test('GET validation failure, cancellation, detached submitter and changed values preserve native protection', async () => {
  for (const failure of ['validation', 'cancel', 'detached', 'values']) {
    const f = fixture(); f.dirty(); const form = f.form(); const button = f.button(form);
    if (failure === 'validation') form.valid = false;
    if (failure === 'cancel') f.document.addEventListener('submit', event => event.preventDefault());
    submit(f, form, button);
    if (failure === 'detached') button.isConnected = false;
    if (failure === 'values') form.elements.push({ name: 'query', value: 'new', type: 'text' });
    f.questions[0].answer(true); await flush();
    assert.equal(f.unload(), true, failure);
  }
});

test('POSTs and GETs into other targets are never mistaken for abandoning navigation', () => {
  const f = fixture(); f.dirty();
  assert.equal(submit(f, f.form({ method: 'post' })).defaultPrevented, false);
  assert.equal(submit(f, f.form({ target: '_blank' })).defaultPrevented, false);
  assert.equal(f.questions.length, 0);
});

test('HTMX full-page GET resumes once without arming a native navigation exception', async () => {
  const f = fixture(); f.dirty();
  assert.equal(htmx(f, f.link()).defaultPrevented, true);
  assert.deepEqual(f.requests, []);
  f.questions[0].answer(true); await flush();
  assert.deepEqual(f.requests, [true]);
  assert.equal(f.questions.length, 1);
  assert.equal(f.unload(), true, 'HTMX abort/no-swap leaves the draft protected');
});

test('HTMX previews, preserving swaps and POST renders never ask to discard', () => {
  const f = fixture(); f.dirty();
  assert.equal(htmx(f, f.link(), { verb: 'post' }).defaultPrevented, false);
  assert.equal(htmx(f, f.link(), { target: { isConnected: true, contains: () => false } }).defaultPrevented, false);
  const link = f.link(); link.closest = () => ({ getAttribute: name => name === 'hx-swap' ? 'none' : null });
  assert.equal(htmx(f, link).defaultPrevented, false);
  assert.equal(f.questions.length, 0);
});

test('HTMX missing translated copy fails closed and keeps the dirty draft guarded', () => {
  const f = fixture(); f.dirty(); delete f.document.body.dataset.msgUnsavedLeave;
  assert.equal(htmx(f, f.link()).defaultPrevented, true);
  assert.equal(f.questions.length, 0);
  assert.deepEqual(f.requests, []);
  assert.equal(f.unload(), true);
});

test('pageshow revokes pending answers and any native navigation exception', async () => {
  const f = fixture(); f.dirty(); click(f, f.link());
  f.pageshow(); f.questions[0].answer(true); await flush();
  assert.equal(f.unload(), true);
  click(f, f.link()); f.questions[1].answer(true); await flush();
  f.pageshow(); assert.equal(f.unload(), true);
});

test('GET overrides on POST forms stay interactive after Cancel and replay with busy loaded first', async () => {
  for (const formmethod of ['get', '']) {
    const f = fixture({ busy: true }); f.dirty();
    const form = f.form({ method: 'post', action: '/ui/editor' });
    const button = f.button(form, { formmethod, formaction: '', formtarget: '' });
    submit(f, form, button); f.questions[0].answer(false); await flush();
    assert.equal(button.attrs['aria-busy'], undefined);
    assert.notEqual(button.disabled, true);
    submit(f, form, button);
    assert.equal(f.questions.length, 2);
    f.questions[1].answer(true); await flush();
    assert.deepEqual(f.requests, [button]);
    assert.equal(button.attrs['aria-busy'], undefined);
    assert.equal(f.unload(), false);
  }
});

test('HTMX preserves a distinct confirmation even without the shared modal', async () => {
  const f = fixture(); f.dirty(); delete f.window.HfsConfirm;
  const prompts = [];
  f.window.confirm = message => { prompts.push(message); return prompts.length === 1; };
  htmx(f, f.link(), { question: 'Proceed with this action?' }); await flush();
  assert.deepEqual(prompts, ['Discard your changes and leave?', 'Proceed with this action?']);
  assert.deepEqual(f.requests, [], 'rejecting the action confirmation cannot send its request');
  assert.equal(f.unload(), true);
});

test('HTMX modified and middle-button activations do not open the navigation modal', () => {
  const f = fixture(); f.dirty();
  for (const triggeringEvent of [{ type: 'click', button: 1 }, { type: 'click', button: 0, ctrlKey: true }]) {
    assert.equal(htmx(f, f.link(), { triggeringEvent }).defaultPrevented, false);
  }
  assert.equal(f.questions.length, 0);
});

test('default trackers check an authored edit before rAF while keeping untouched bootstrap clean', async () => {
  const f = fixture();
  const root = f.form(); let value = '';
  const tracker = f.window.HfsUnsaved.track({ root, read: () => value });
  value = '{}';
  assert.equal(click(f, f.link()).defaultPrevented, false);
  tracker.reset(); value = '{"active":true}';
  root.listeners.find(l => l.type === 'input').fn({});
  assert.equal(tracker.isDirty(), false, 'no frame has run yet');
  assert.equal(click(f, f.link()).defaultPrevented, true);
  f.questions[0].answer(false); await flush();
  assert.equal(f.unload(), true);
});

test('focus-induced trusted change settles the edit that the dialog is already asking about', async () => {
  const f = fixture(); f.set('{"active":false}\n--pending--\nactive="true"');
  const link = f.link();
  link.focus = () => { f.dispatch('change', { target: { parent: f.root }, isTrusted: true }); };
  click(f, link); f.set('{"active":true}');
  f.questions[0].answer(true); await flush();
  assert.equal(f.questions.length, 1);
  assert.equal(f.unload(), false);
});

test('the shared hx-confirm listener respects a prior navigation decision and loads once', async () => {
  const document = { listeners: [], body: { dataset: {} },
    addEventListener(type, fn) { this.listeners.push({ type, fn }); } };
  let prompts = 0;
  const window = { confirm() { prompts++; return true; } };
  const context = vm.createContext({ window, document, Promise });
  const confirmSource = fs.readFileSync(path.resolve(__dirname, '../../assets/confirm.js'), 'utf8');
  vm.runInContext(confirmSource, context); vm.runInContext(confirmSource, context);
  assert.equal(document.listeners.filter(l => l.type === 'htmx:confirm').length, 1);
  const listener = document.listeners.find(l => l.type === 'htmx:confirm').fn;
  listener({ defaultPrevented: true, detail: { question: 'Leave?' } });
  await flush();
  assert.equal(prompts, 0);
  let issued = 0;
  listener({ defaultPrevented: false, preventDefault() {}, detail: { question: 'Delete?', issueRequest() { issued++; } } });
  await flush();
  assert.equal(prompts, 1);
  assert.equal(issued, 1);
});

test('a tracked GET into another tab does not disable protection for the current draft', () => {
  const f = fixture(); const form = f.form({ target: '_blank' });
  let value = 'loaded';
  f.window.HfsUnsaved.track({ root: form, form, read: () => value, checkOnExit: true });
  value = 'draft';
  assert.equal(submit(f, form).defaultPrevented, false);
  assert.equal(f.questions.length, 0);
  assert.equal(f.unload(), true);
});

test('failed-Save documents start protected, navigation keeps their draft dirty, and reset releases it', async () => {
  const f = fixture(); const form = f.form({ method: 'post', 'data-unsaved-draft': '' });
  let value = '{"resourceType":"ViewDefinition","resource":"Nope"}';
  const tracker = f.window.HfsUnsaved.track({ root: form, form, read: () => value, checkOnExit: true });
  assert.equal(tracker.isDirty(), true, 'the re-rendered initial draft has not been persisted');
  assert.equal(f.unload(), true);
  click(f, f.link()); f.questions[0].answer(false); await flush();
  assert.equal(tracker.isDirty(), true);
  click(f, f.link()); f.questions[1].answer(true); await flush();
  assert.equal(tracker.isDirty(), true, 'navigation authorization never cleans a failed Save');
  assert.equal(f.unload(), false);
  assert.equal(f.unload(), true);
  tracker.markClean();
  assert.equal(tracker.check(), false, 'existing in-page discard still acknowledges exactly this draft');
  value = '{"resourceType":"ViewDefinition","resource":"Patient"}';
  assert.equal(tracker.check(), true, 'later edits restore protection');
  tracker.reset();
  assert.equal(tracker.check(), false, 'a genuine document reset releases the failed-save state');
});

for (const [name, change] of [
  ['destination', (f, link) => { link.attrs.href = '/ui/history'; }],
  ['source', (f, link) => { link.isConnected = false; }],
  ['target', f => { f.document.body.isConnected = false; }],
]) {
  test(`HTMX distinct confirmation revalidates the ${name} before issuing`, async () => {
    const f = fixture(); f.dirty(); const link = f.link();
    htmx(f, link, { question: 'Proceed?' });
    f.questions[0].answer(true); await flush();
    assert.equal(f.questions[1].message, 'Proceed?');
    change(f, link); f.questions[1].answer(true); await flush();
    assert.deepEqual(f.requests, [], 'the distinct confirmation cannot authorize a stale navigation');
    assert.equal(f.unload(), true);
  });
}

test('HTMX a draft changed during its distinct confirmation requires a fresh discard decision', async () => {
  const f = fixture(); f.dirty();
  htmx(f, f.link(), { question: 'Proceed?' });
  f.questions[0].answer(true); await flush();
  f.set('{"active":true,"name":"newer"}');
  f.questions[1].answer(true); await flush();
  assert.deepEqual(f.requests, []);
  assert.equal(f.questions.length, 3);
  assert.equal(f.questions[2].message, 'Discard your changes and leave?');
  f.questions[2].answer(true); await flush();
  assert.deepEqual(f.requests, [true], 'the earlier action confirmation and fresh discard resume once');
  assert.equal(f.unload(), true, 'HTMX still receives no native navigation permission');
});

test('HTMX pending settlement during its distinct confirmation remains the same discard decision', async () => {
  const f = fixture(); f.set('{"active":false}\n--pending--\nactive="true"');
  htmx(f, f.link(), { question: 'Proceed?' });
  f.questions[0].answer(true); await flush();
  f.set('{"active":true}');
  f.questions[1].answer(true); await flush();
  assert.equal(f.questions.length, 2);
  assert.deepEqual(f.requests, [true]);
  assert.equal(f.unload(), true);
});

test('pageshow revokes a second HTMX confirmation and a fresh lifecycle attempt can proceed', async () => {
  const f = fixture(); f.dirty(); const link = f.link();
  htmx(f, link, { question: 'Proceed?' });
  f.questions[0].answer(true); await flush();
  assert.equal(f.questions[1].message, 'Proceed?');
  f.pageshow(); f.set('{"active":true,"name":"restored_draft"}');
  f.questions[1].answer(true); await flush();
  assert.deepEqual(f.requests, [], 'a restored lifecycle cannot use the earlier discard permission');
  assert.equal(f.questions.length, 2, 'the stale answer must not open another discard question');
  assert.equal(f.window.HfsUnsaved.isDirty(), true);
  assert.equal(f.unload(), true);
  htmx(f, link, { question: 'Proceed?' });
  f.questions[2].answer(true); await flush();
  f.questions[3].answer(true); await flush();
  assert.deepEqual(f.requests, [true], 'a new lifecycle decision resumes exactly once');
  assert.equal(f.unload(), true, 'HTMX never consumes native protection');
  f.set('{"active":false}');
  assert.equal(f.window.HfsUnsaved.isDirty(), false, 'the loaded baseline survives revocation and recovery');
});
