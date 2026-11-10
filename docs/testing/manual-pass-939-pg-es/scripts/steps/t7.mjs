// T7 -- $sql-export (matrix section 11, rows 7.a-7.q). Run: cd /home/edson/tmp/939/pw && node steps/t7.mjs
// Env: HFS_BASE, OUT (see lib.mjs); PID (anchor patient id), T3_START / T3_END (UTC instants, rows 7.i/7.j),
//   GROUP_ID (default manual-group), EXPECT_PATIENTS / EXPECT_FEMALE (override counts read from the API),
//   T7_PID_SEARCH (text typed in the Patients combobox, default Cari853), T7_JOB_TIMEOUT_MIN (default 20),
//   T7_BIG (1/0; default: on when Observation count >= 1,000,000) + T7_BIG_TIMEOUT_MIN (default 90),
//   T7_RESTART_CMD (shell command that restarts HFS; without it the restart row is recorded na),
//   T7_PY (python with pyarrow; default scratch venv, else python3), T7_DL (download dir, default $OUT/dl).
import { launch, shot, record, api, BASE, OUT, OUTPUTS } from '../lib.mjs';
import fs from 'fs';
import path from 'path';
import { execFileSync, execSync } from 'child_process';
import { Readable } from 'stream';
import { pipeline } from 'stream/promises';
import readline from 'readline';
import { fileURLToPath } from 'url';

const S = 'T7';
const env = process.env;
const HERE = path.dirname(fileURLToPath(import.meta.url));
const PID = env.PID || '7d24f7a0-6f2e-ce3b-5568-db7b14695583';
const GROUP_ID = env.GROUP_ID || 'manual-group';
const T3_START = env.T3_START || '';
const T3_END = env.T3_END || '';
const JOB_TIMEOUT = Number(env.T7_JOB_TIMEOUT_MIN || 20) * 60_000;
const DL = env.T7_DL || path.join(OUT, 'dl');
const PY = env.T7_PY || (fs.existsSync(`${env.HOME}/tmp/939/scratch-t7/pyvenv/bin/python`) ? `${env.HOME}/tmp/939/scratch-t7/pyvenv/bin/python` : 'python3');
fs.mkdirSync(DL, { recursive: true });

const recorded = new Set();
function rec(id, result, note = '') { recorded.add(id); record(S, id, result, note); }
const pf = (ok, note) => ({ ok, note });
async function row(id, fn) {
  try {
    const r = await fn();
    if (r && r.skip) rec(id, 'na', r.skip);
    else if (r && 'ok' in r) rec(id, r.ok ? 'pass' : 'fail', r.note);
    else if (!recorded.has(id)) rec(id, 'fail', 'row function returned nothing');
  } catch (e) { rec(id, 'fail', `exception: ${String(e.message || e).split('\n')[0]}`); }
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const J = (o) => JSON.stringify(o);

// ---------------------------------------------------------------- data helpers
async function total(q) {
  const r = await api(`${q}${q.includes('?') ? '&' : '?'}_summary=count&_total=accurate`);
  return r.json && typeof r.json.total === 'number' ? r.json.total : null;
}
async function findId(type, query) {
  const r = await api(`/${type}?${query}&_count=5`);
  const e = r.json?.entry?.find((x) => x.resource);
  return e ? e.resource.id : null;
}
async function post(type, body) {
  const r = await api(`/${type}`, { method: 'POST', headers: { 'Content-Type': 'application/fhir+json' }, body: J(body) });
  return r.json?.id || null;
}
const PD_VD = { resourceType: 'ViewDefinition', url: 'http://example.org/ViewDefinition/patient_demographics', name: 'patient_demographics', status: 'active', resource: 'Patient',
  select: [{ column: [{ name: 'id', path: 'getResourceKey()', type: 'id' }, { name: 'gender', path: 'gender' }, { name: 'birth_date', path: 'birthDate', type: 'date' }, { name: 'family', path: 'name.first().family' }, { name: 'city', path: 'address.first().city' }] }],
  where: [{ path: 'active.exists().not() or active = true' }] };
const OBS_VD = { resourceType: 'ViewDefinition', url: 'http://example.org/ViewDefinition/observation_flat', name: 'observation_flat', status: 'active', resource: 'Observation',
  select: [{ column: [{ name: 'id', path: 'getResourceKey()', type: 'id' }, { name: 'patient_id', path: 'subject.getReferenceKey(Patient)', type: 'id' }, { name: 'code', path: 'code.coding.first().code' }, { name: 'value', path: 'value.ofType(Quantity).value', type: 'decimal' }, { name: 'effective', path: 'effective.ofType(dateTime)', type: 'dateTime' }] }] };
const VIEW_JSON = { resourceType: 'Library', name: 'female_patients', status: 'active', url: 'http://example.org/Library/female_patients',
  type: { coding: [{ system: 'http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes', code: 'sql-view' }] },
  relatedArtifact: [{ type: 'depends-on', resource: 'http://example.org/ViewDefinition/patient_demographics', label: 'pd' }] };
const VIEW_SQL = "SELECT id, birth_date, city FROM pd WHERE gender = 'female'";
const QUERY_JSON = { resourceType: 'Library', name: 'tall_female_patients', status: 'active', url: 'http://example.org/Library/tall_female_patients',
  type: { coding: [{ system: 'http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes', code: 'sql-query' }] },
  relatedArtifact: [{ type: 'depends-on', resource: 'http://example.org/ViewDefinition/observation_flat', label: 'obs' }, { type: 'depends-on', resource: 'http://example.org/Library/female_patients', label: 'fp' }],
  parameter: [{ name: 'min_height', use: 'in', type: 'decimal' }] };
const QUERY_SQL = "SELECT fp.id, fp.city, MAX(obs.value) AS height\nFROM fp JOIN obs ON obs.patient_id = fp.id\nWHERE obs.code = '8302-2' AND obs.value > :min_height\nGROUP BY fp.id, fp.city";
const BROKEN_JSON = { resourceType: 'Library', name: 'broken_query', status: 'active', url: 'http://example.org/Library/broken_query',
  type: { coding: [{ system: 'http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes', code: 'sql-query' }] },
  relatedArtifact: [{ type: 'depends-on', resource: 'http://example.org/ViewDefinition/patient_demographics', label: 'pd' }] };

// ---------------------------------------------------------------- UI helpers
const { browser, context, page } = await launch({ permissions: ['clipboard-read', 'clipboard-write'] });
page.on('dialog', (d) => d.accept().catch(() => {}));
page.setDefaultTimeout(20_000);
const go = async (p) => { await page.goto(p.startsWith('http') ? p : BASE + p, { waitUntil: 'load' }); };

/** Answer the shared in-page confirmation dialog (HfsConfirm); returns its message or null when none appeared. */
async function confirmDialog(pg = page, accept = true) {
  const d = pg.locator('dialog.confirm-dialog[open]');
  try { await d.waitFor({ state: 'visible', timeout: 4000 }); } catch { return null; }
  const msg = (await d.getAttribute('aria-label')) || (await d.innerText());
  const btns = d.locator('button');
  const n = await btns.count();
  const pick = accept ? (await d.locator('button.btn--danger, button.btn--primary').count() ? d.locator('button.btn--danger, button.btn--primary').first() : btns.nth(n - 1)) : btns.first();
  await pick.click();
  return msg.trim();
}
async function visibleErrors(pg = page) {
  return pg.$$eval('[role=alert], .field__hint--error, .notice--warn', (els) =>
    els.filter((e) => !e.hidden && e.offsetParent !== null && e.textContent.trim()).map((e) => e.textContent.trim().replace(/\s+/g, ' ')));
}
async function setCM(cm, textarea, text) {
  let used = 'codemirror';
  try {
    await cm.waitFor({ state: 'visible', timeout: 6000 });
    await cm.click();
    await page.keyboard.press('Control+a');
    await page.keyboard.press('Delete');
    await page.keyboard.insertText(text);
  } catch { used = 'textarea'; }
  await sleep(300);
  const v = await textarea.inputValue();
  if (used === 'textarea' || (text.length < 4000 && !v.includes(text.slice(0, Math.min(40, text.length))))) {
    await textarea.evaluate((el, t) => { el.value = t; el.dispatchEvent(new Event('input', { bubbles: true })); }, text);
    used = 'textarea-forced';
  }
  return used;
}
const jsonCM = () => page.locator('.card:has(textarea[name="json"]) .cm-content').first();
const sqlCM = () => page.locator(".sql-editor .cm-content[role='textbox']").first();

/** Open the editor in Create New mode, replace JSON then SQL, return {how, preview}. */
async function fillLibrary(kind, jsonObj, sql) {
  await go(`/ui/sql/${kind}?lib=new`);
  await page.locator('textarea[name="sql"]').waitFor({ state: 'attached' });
  const h1 = await setCM(jsonCM(), page.locator('textarea[name="json"]'), JSON.stringify(jsonObj));
  await sleep(700);
  const h2 = await setCM(sqlCM(), page.locator('textarea[name="sql"]'), sql);
  await sleep(2500);
  return `${h1}/${h2}`;
}
async function previewText() { return (await page.locator('#run-notice, #run-results').allInnerTexts()).join('\n').replace(/[ \t]+/g, ' '); }
async function saveLibrary() {
  await page.locator("button[name='action'][value='save']").click();
  await page.waitForLoadState('load');
  await sleep(500);
  const url = page.url();
  const id = (url.match(/[?&]lib=([^&]+)/) || [])[1] || null;
  return { id, saved: /saved=1/.test(url), notice: (await page.locator('.notice').allInnerTexts()).join(' | '), url };
}

// ---------------------------------------------------------------- export helpers
async function listCards() {
  await go('/ui/sql/export');
  return page.$$eval('.job-card', (els) => els.map((e) => ({
    id: e.id.replace(/^job-/, ''),
    name: (e.querySelector('.job-card__name')?.textContent || '').trim(),
    status: ((e.querySelector('.job-card__actions .tag')?.className || '').match(/tag--(\S+)/) || [])[1] || '',
    label: (e.querySelector('.job-card__actions .tag')?.textContent || '').trim(),
    meta: (e.querySelector('.job-card__meta')?.textContent || '').replace(/\s+/g, ' ').trim(),
  })));
}
/** Fill and submit /ui/sql/export/new. spec: name, subjects[], params{subject:{p:v}}, format, header, patient, group, sincePreset, sinceCustom, tracking. */
async function newExport(spec, pg = page, { submit = true, js = true } = {}) {
  await pg.goto(`${BASE}/ui/sql/export/new`, { waitUntil: 'load' });
  await pg.locator('form[action="/ui/sql/export"]').waitFor();
  if (spec.name !== undefined) await pg.locator('input[name="name"]').fill(spec.name);
  for (const s of spec.subjects || []) await pg.locator(`tr[data-name="${s}"] input[name="subject"]`).check();
  for (const [s, ps] of Object.entries(spec.params || {})) {
    const ref = await pg.locator(`tr[data-name="${s}"] input[name="subject"]`).getAttribute('value');
    for (const [k, v] of Object.entries(ps)) await pg.locator(`tr.row--params[data-subject="${ref}"] [name="param:${ref}:${k}"]`).fill(v);
  }
  if (spec.patient) await pickCombo(pg, 'sql-export-patients', 'patient', spec.patientSearch || spec.patient, spec.patient, js);
  if (spec.group) await pickCombo(pg, 'sql-export-groups', 'group', spec.group, spec.group, js);
  if (spec.sincePreset) {
    if (js) {
      await pg.locator('#sql-export-since button[role="combobox"]').click();
      await pg.locator(`#sql-export-since [role="option"][data-value="${spec.sincePreset}"]`).click();
    } else await pg.selectOption('select[name="since_preset"]', spec.sincePreset);
  }
  if (spec.sinceCustom !== undefined) await pg.locator('input[name="since_custom"]').fill(spec.sinceCustom);
  if (spec.format) {
    const r = pg.locator(`input[name="format"][value="${spec.format}"]`);
    try { await r.check({ timeout: 3000 }); } catch { await r.check({ force: true }); }
  }
  if (spec.header !== undefined) await pg.locator('input[name="header"]').setChecked(spec.header, { force: true });
  if (spec.tracking !== undefined) {
    const det = pg.locator('details:has(input[name="client_tracking_id"])');
    if (js) await det.locator('summary').click().catch(() => {});
    else await det.evaluate((d) => { d.open = true; }).catch(() => {});
    await pg.locator('input[name="client_tracking_id"]').fill(spec.tracking);
  }
  if (!submit) return { submitted: false };
  await pg.locator("form[action='/ui/sql/export'] button[type='submit']").click();
  await pg.waitForLoadState('load');
  await sleep(400);
  const stillForm = await pg.locator('form.bulk-export-form').count();
  return { submitted: !stillForm, errors: await visibleErrors(pg), url: pg.url() };
}
async function pickCombo(pg, rootId, field, search, value, js) {
  const root = pg.locator(`#${rootId}`);
  if (!js) { await root.locator(`textarea[name="${field}"]`).fill(value); return; }
  const box = root.getByRole('combobox');
  await box.fill(search);
  const opt = root.locator(`[data-combobox-option][data-value$="/${value}"], [data-combobox-option][data-value="${value}"]`).first();
  try { await opt.waitFor({ state: 'visible', timeout: 15_000 }); await opt.click(); }
  catch {
    // fall back: first option whose label mentions the search text
    const any = root.locator('[data-combobox-option]').first();
    await any.waitFor({ state: 'visible', timeout: 5000 });
    await any.click();
  }
}
/** Poll the list until the (newest) card called `name` not in `skip` is terminal. */
async function waitJob(name, { skip = new Set(), timeout = JOB_TIMEOUT, wantStatus = null } = {}) {
  const t0 = Date.now(); let seenInProgress = null; let last = null;
  while (Date.now() - t0 < timeout) {
    const cards = await listCards();
    const c = cards.find((x) => x.name === name && !skip.has(x.id));
    if (c) {
      last = c;
      if (c.status === 'in-progress') seenInProgress = c.meta;
      if (wantStatus ? c.status === wantStatus : ['complete', 'failed', 'cancelled'].includes(c.status)) return { card: c, seenInProgress };
    }
    await sleep(c?.status === 'in-progress' ? 4000 : 1500);
  }
  return { card: last, seenInProgress, timedOut: true };
}
async function detailOf(id) {
  await go(`/ui/sql/export/${id}`);
  await page.locator('#job-detail').waitFor();
  return page.evaluate(() => {
    const facts = {};
    for (const f of document.querySelectorAll('.detail__field')) {
      const k = f.querySelector('span')?.textContent.trim(); const v = f.querySelector('div')?.textContent.replace(/\s+/g, ' ').trim();
      if (k) facts[k] = v;
    }
    const tags = (label) => [...document.querySelectorAll('.detail__field')].filter((f) => f.querySelector('span')?.textContent.trim() === label).flatMap((f) => [...f.querySelectorAll('.tag')].map((t) => t.textContent.trim()));
    const outputs = [...document.querySelectorAll('.table-card tbody tr:not(.data-table__empty)')].map((tr) => ({
      name: tr.children[0]?.textContent.trim(), subject: tr.children[1]?.textContent.replace(/\s+/g, ' ').trim(),
      files: [...tr.querySelectorAll('a[download]')].map((a) => ({ href: a.getAttribute('href'), label: a.getAttribute('download') || a.textContent.trim() })),
    }));
    return {
      title: document.querySelector('h1')?.textContent.trim(), lede: document.querySelector('.page-head__lede')?.textContent.replace(/\s+/g, ' ').trim(),
      status: document.querySelector('.page-head__action .tag')?.textContent.trim(),
      warning: document.querySelector('.job-detail-stack .notice--warn')?.textContent.replace(/\s+/g, ' ').trim() || '',
      facts, subjectsText: document.querySelector('.job-detail__subjects')?.textContent.replace(/\s+/g, ' ').trim() || '',
      subjectChips: [...document.querySelectorAll('.job-detail__subjects .tag--param')].map((t) => t.textContent.trim()),
      patients: tags('Patients'), groups: tags('Groups'),
      jobId: document.querySelector('.detail__field code')?.textContent.trim() || '', outputs,
    };
  });
}
async function download(href, label, dir) {
  const u = new URL(href, BASE);
  const url = BASE + u.pathname + u.search;
  fs.mkdirSync(dir, { recursive: true });
  const dest = path.join(dir, label);
  const res = await fetch(url);
  if (!res.ok) throw new Error(`download ${label}: HTTP ${res.status}`);
  await pipeline(Readable.fromWeb(res.body), fs.createWriteStream(dest));
  return { label, path: dest, size: fs.statSync(dest).size };
}
async function readLines(file, max = Infinity) {
  const out = []; let n = 0;
  const rl = readline.createInterface({ input: fs.createReadStream(file), crlfDelay: Infinity });
  for await (const line of rl) { if (line === '') continue; n++; if (out.length < max) out.push(line); }
  return { n, lines: out };
}
function csvParse(line) {
  const out = []; let cur = ''; let q = false;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (q) { if (c === '"' && line[i + 1] === '"') { cur += '"'; i++; } else if (c === '"') q = false; else cur += c; }
    else if (c === '"') q = true; else if (c === ',') { out.push(cur); cur = ''; } else cur += c;
  }
  out.push(cur); return out;
}
function pq(file, sampleOut) {
  const out = execFileSync(PY, ['-I', path.join(HERE, 't7_pq.py'), file, sampleOut || ''], { encoding: 'utf8', maxBuffer: 1 << 26 });
  return JSON.parse(out.trim().split('\n').pop());
}
const EXT = { ndjson: 'ndjson', csv: 'csv', json: 'json', parquet: 'parquet' };
function saveSample(fmt, name, f) {
  try {
    const dest = path.join(OUTPUTS, `t7-sample-${fmt}-${name}.${EXT[fmt]}`);
    if (fmt === 'ndjson' || fmt === 'csv') {
      const text = fs.readFileSync(f, 'utf8').split('\n').slice(0, 21).join('\n');
      fs.writeFileSync(dest, text.endsWith('\n') ? text : text + '\n');
    } else if (fmt === 'json') {
      const fd = fs.openSync(f, 'r'); const buf = Buffer.alloc(Math.min(8000, fs.statSync(f).size)); fs.readSync(fd, buf, 0, buf.length, 0); fs.closeSync(fd);
      fs.writeFileSync(dest, buf.toString('utf8') + (fs.statSync(f).size > 8000 ? '\n... [truncated sample: first 8000 bytes]\n' : ''));
    } else if (fmt === 'parquet') pq(f, dest);
  } catch (e) { console.log('sample save failed', e.message); }
}
async function saveManifest(name, jobId) {
  if (!jobId) return null;
  const r = await api(`/export/${jobId}/status`, { save: `t7-manifest-${name}.json` });
  return r;
}

/** Full cycle: submit through the UI, wait, screenshot, open detail, download all files. */
async function runExport(spec, { shotName, timeout = JOB_TIMEOUT, download: doDl = true, skipIds = new Set() } = {}) {
  const before = new Set([...skipIds, ...(await listCards()).map((c) => c.id)]);
  const sub = await newExport(spec);
  const res = { spec, sub };
  if (!sub.submitted) return res;
  await sleep(300);
  const w = await waitJob(spec.name, { skip: before, timeout });
  res.card = w.card; res.seen = w.seenInProgress; res.timedOut = w.timedOut;
  if (shotName) await shot(page, S, `${shotName}-card`);
  if (!w.card) return res;
  res.det = await detailOf(w.card.id);
  if (shotName) await shot(page, S, `${shotName}-detail`, { full: true });
  res.manifest = await saveManifest(spec.name, res.det.jobId);
  res.files = [];
  if (doDl && w.card.status === 'complete') {
    for (const o of res.det.outputs) for (const f of o.files) {
      try { res.files.push({ ...(await download(f.href, f.label, path.join(DL, spec.name))), subject: o.subject }); }
      catch (e) { res.files.push({ label: f.label, error: e.message }); }
    }
  }
  return res;
}
const sumNote = (r) => `card=${r.card ? `${r.card.status}/${r.card.label}` : 'none'}${r.det?.warning ? ` warning="${r.det.warning}"` : ''}${r.timedOut ? ' TIMED OUT' : ''}${r.sub && !r.sub.submitted ? ` form-errors=${J(r.sub.errors)}` : ''}`;
const okFiles = (r) => r.card?.status === 'complete' && r.files?.length && r.files.every((f) => !f.error);

// ---------------------------------------------------------------- expectations from the server
const N_PAT = Number(env.EXPECT_PATIENTS) || (await total('/Patient'));
const N_FEM = Number(env.EXPECT_FEMALE) || (await total('/Patient?gender=female'));
const N_OBS = await total('/Observation');
console.log(`expected: patients=${N_PAT} female=${N_FEM} observations=${N_OBS} pid=${PID} py=${PY}`);
const MATRIX_PAT = 11705, MATRIX_FEM = 5814;
const cmp = (got, exp, matrix) => `${got} (expected ${exp}${matrix !== undefined && matrix !== exp ? `; matrix value ${matrix} is for the full corpus` : ''})`;

// ---------------------------------------------------------------- setup: ViewDefinitions, SQL View, SQL Query (11.1, 11.2)
let QV = null, QQ = null;
await row('7.setup-vd', async () => {
  const msgs = [];
  for (const [vd, nm] of [[PD_VD, 'patient_demographics'], [OBS_VD, 'observation_flat']]) {
    let id = await findId('ViewDefinition', `name=${nm}`);
    if (id) msgs.push(`${nm} exists (${id})`); else { id = await post('ViewDefinition', vd); msgs.push(`${nm} created via API (${id}) -- T6 normally creates it`); }
    if (!id) return pf(false, `could not obtain ${nm}`);
  }
  return pf(true, msgs.join('; '));
});

await row('7.setup-view', async () => {
  let id = await findId('Library', 'name=female_patients');
  if (id) { QV = id; return pf(true, `female_patients already exists (QV=${id}), UI creation skipped`); }
  const how = await fillLibrary('views', VIEW_JSON, VIEW_SQL);
  const prev = await previewText();
  await shot(page, S, '11.1-sql-view-preview');
  const r = await saveLibrary();
  QV = r.id;
  await shot(page, S, '11.1-sql-view-saved');
  const rail = await page.locator('.filter-rail__item[aria-current="true"]').first().innerText().catch(() => '');
  const ok = r.saved && /Saved\./.test(r.notice) && /female_patients/.test(rail) && /active/.test(rail) && /\b50 rows/.test(prev) && /birth_date/i.test(prev) && /city/i.test(prev);
  return pf(ok, `editor=${how}; saved=${r.saved} notice="${r.notice}" rail="${rail.replace(/\s+/g, ' ')}" QV=${QV}; preview="${prev.replace(/\s+/g, ' ').slice(0, 160)}"`);
});

await row('7.setup-query', async () => {
  let id = await findId('Library', 'name=tall_female_patients');
  if (id) { QQ = id; return pf(true, `tall_female_patients already exists (QQ=${id}), UI creation skipped`); }
  const how = await fillLibrary('queries', QUERY_JSON, QUERY_SQL);
  const waiting = await previewText();
  const field = page.locator('input[name="min_height"], #lib-params input[name*="min_height"]').first();
  const hasField = await field.count();
  await shot(page, S, '11.2-sql-query-waiting');
  let after = '';
  if (hasField) { await field.fill('150'); await sleep(3000); after = await previewText(); }
  await shot(page, S, '11.2-sql-query-min-height');
  const r = await saveLibrary();
  QQ = r.id;
  await shot(page, S, '11.2-sql-query-saved');
  const chips = await page.locator('.page-head__title--kind .tag').allInnerTexts().catch(() => []);
  const okWait = /Waiting for a value for :min_height/.test(waiting);
  const ok = r.saved && okWait && !!hasField && chips.some((c) => /SQL Query/i.test(c)) && chips.some((c) => /active/i.test(c));
  return pf(ok, `editor=${how}; waitingMsg=${okWait ? 'yes' : `NO ("${waiting.replace(/\s+/g, ' ').slice(0, 140)}")`}; paramField=${!!hasField}; chips=${J(chips)}; saved=${r.saved} QQ=${QQ}; preview150="${after.replace(/\s+/g, ' ').slice(0, 140)}"`);
});

await row('7.setup-query-neg', async () => {
  if (!QQ) return { skip: 'query not created' };
  await go(`/ui/sql/queries?lib=${QQ}`);
  const cur = JSON.parse(await page.locator('textarea[name="json"]').inputValue());
  cur.type.coding[0].code = 'sql-view';
  await setCM(jsonCM(), page.locator('textarea[name="json"]'), JSON.stringify(cur));
  await sleep(800);
  const r = await saveLibrary();
  const errs = await visibleErrors();
  await shot(page, S, '11.2-sql-query-neg-type');
  const want = 'The Library\'s SQL on FHIR type must be "sql-query" to save it here.';
  const seen = errs.join(' | ');
  // restore: reload the saved library (discard the unsaved edit)
  await go(`/ui/sql/queries?lib=${QQ}`);
  await confirmDialog().catch(() => {});
  const stored = await api(`/Library/${QQ}`);
  const code = stored.json?.type?.coding?.[0]?.code;
  return pf(seen.includes(want) && code === 'sql-query', `on-screen="${seen}"; stored type code after attempt=${code}`);
});

// ---------------------------------------------------------------- 11.3 / 11.4 main grid
const R = {}; // results by job name
const recs = [];
const fmtCheck = {
  ndjson: async (f) => { const { n, lines } = await readLines(f.path, 1e6); const objs = lines.map((l) => JSON.parse(l)); return { n, keys: Object.keys(objs[0] || {}), objs }; },
  csv: async (f) => { const { n, lines } = await readLines(f.path, 1e6); return { n, header: lines[0], data: lines.slice(1) }; },
  json: async (f) => { const t = fs.statSync(f.path).size > 300e6 ? null : JSON.parse(fs.readFileSync(f.path, 'utf8')); return { isArray: Array.isArray(t), len: t?.length, first: t?.[0] }; },
  parquet: async (f) => pq(f.path),
};
const subjName = (fileLabel) => fileLabel;

await row('7.a', async () => {
  const r = R.a = await runExport({ name: 'vd-ndjson', subjects: ['patient_demographics'], format: 'ndjson' }, { shotName: '7.a-vd-ndjson' });
  if (!okFiles(r)) return pf(false, sumNote(r));
  saveSample('ndjson', 'vd', r.files[0].path);
  const c = await fmtCheck.ndjson(r.files[0]);
  const want = ['id', 'gender', 'birth_date', 'family', 'city'];
  const pidLine = c.objs.find((o) => o.id === PID);
  const dur = r.det.facts['Duration'] || r.det.facts[Object.keys(r.det.facts).find((k) => /duration|finished/i.test(k))] || J(r.det.facts);
  R.a.lines = c.n;
  // the list card for the "SQL Exports list" screenshot
  await go('/ui/sql/export'); await shot(page, S, '7.a-sql-exports-list');
  const pidNote = pidLine ? `PID line=${J(pidLine)}` : 'PID not in file';
  return pf(c.n === N_PAT && want.every((k) => c.keys.includes(k)) && r.files.length === 1, `files=${r.files.length} lines=${cmp(c.n, N_PAT, MATRIX_PAT)} keys=${c.keys} finished-in="${dur}" ${pidNote}`);
});

// -- view the card's "View files" button on the list for 7.a (UI path)
await row('7.a-viewfiles', async () => {
  if (!R.a?.card) return { skip: '7.a produced no card' };
  await go('/ui/sql/export');
  await page.locator(`#job-${R.a.card.id}`).getByRole('link', { name: 'View files' }).click();
  await page.waitForLoadState('load');
  const facts = await page.locator('.detail__field').allInnerTexts();
  await shot(page, S, '7.a-view-files');
  return pf(/\/ui\/sql\/export\/[^/]+$/.test(page.url()) && facts.some((x) => /Job/i.test(x)), `url=${page.url()} cards=${facts.length}`);
});

await row('7.b', async () => {
  const r = R.b = await runExport({ name: 'query-csv', subjects: ['tall_female_patients'], params: { tall_female_patients: { min_height: '150' } }, format: 'csv', header: true }, { shotName: '7.b-query-csv' });
  if (!okFiles(r)) return pf(false, sumNote(r) + ' (a failure naming dependency observation_flat with a 1000000 cap = HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD, 422)');
  saveSample('csv', 'query', r.files[0].path);
  const c = await fmtCheck.csv(r.files[0]);
  const hi = c.header.split(',').indexOf('height');
  const heights = c.data.map((l) => Number(csvParse(l)[hi]));
  const allGt = heights.every((h) => h > 150);
  R.b.data = c.data.length; R.b.rows = c.data;
  const ids = c.data.map((l) => csvParse(l)[0]).slice(0, 2);
  const checks = [];
  for (const id of ids) { const p = await api(`/Patient?_id=${id}`); checks.push(`${id}=${p.json?.entry?.[0]?.resource?.gender}`); }
  const facts = r.det.facts;
  const fmtLabel = facts['Format'];
  const chip = r.det.subjectChips.join(',');
  return pf(c.header === 'id,city,height' && allGt && checks.every((x) => x.endsWith('=female')) && /CSV · with header row/.test(fmtLabel || '') && chip.includes(':min_height = 150') && r.files.length === 1,
    `header="${c.header}" dataRows=${c.data.length} allHeight>150=${allGt} spot=${checks.join(',')} format="${fmtLabel}" chips="${chip}" files=${r.files.length}`);
});

await row('7.c', async () => {
  const r = R.c = await runExport({ name: 'view-parquet', subjects: ['female_patients'], format: 'parquet' }, { shotName: '7.c-view-parquet' });
  if (!okFiles(r)) return pf(false, sumNote(r));
  saveSample('parquet', 'view', r.files[0].path);
  const p = await fmtCheck.parquet(r.files[0]);
  const names = p.schema.map((s) => s[0]).join(',');
  R.c.rows = p.rows;
  return pf(names === 'id,birth_date,city' && p.rows === N_FEM && r.files.length === 1, `files=${r.files.length} schema=${J(p.schema)} rows=${cmp(p.rows, N_FEM, MATRIX_FEM)}`);
});

await row('7.d', async () => {
  const r = R.d = await runExport({ name: 'all-three-json', subjects: ['patient_demographics', 'tall_female_patients', 'female_patients'], params: { tall_female_patients: { min_height: '150' } }, format: 'json' }, { shotName: '7.d-all-three-json' });
  if (!okFiles(r)) return pf(false, sumNote(r));
  const parts = [];
  let allArr = true;
  for (const f of r.files) { const c = await fmtCheck.json(f); parts.push(`${f.label}:${c.isArray ? c.len : 'NOT-ARRAY'}`); allArr &&= c.isArray; }
  const first = r.files.find((f) => /patient_demographics/.test(f.label)) || r.files[0];
  saveSample('json', 'all-three', first.path);
  const labelsOk = r.files.length === 3 && r.files.every((f) => /^all-three-json_.+\.json$/.test(f.label));
  return pf(labelsOk && allArr, `files=${r.files.length} ${parts.join(' ')}`);
});

await row('7.e', async () => {
  const before = new Set((await listCards()).map((c) => c.id));
  const sub = await newExport({ name: 'cancel-me', subjects: ['observation_flat'], format: 'ndjson' });
  if (!sub.submitted) return pf(false, `form not accepted: ${J(sub.errors)}`);
  const cards = await listCards();
  const c = cards.find((x) => x.name === 'cancel-me' && !before.has(x.id));
  if (!c) return pf(false, 'no cancel-me card on the list');
  if (c.status !== 'in-progress') {
    await shot(page, S, '7.e-cancel-me-finished-too-fast');
    return { skip: `job already ${c.status} when the list loaded (${c.meta}); data too small to catch it In progress` };
  }
  await shot(page, S, '7.e-in-progress');
  await page.locator(`#job-${c.id} form[action$="/cancel"] button`).click();
  await page.waitForLoadState('load');
  const w = await waitJob('cancel-me', { skip: before, timeout: 120_000 });
  await shot(page, S, '7.e-cancelled');
  return pf(w.card?.status === 'cancelled', `before: ${c.label} "${c.meta}"; after: ${w.card?.label} "${w.card?.meta}"`);
});

await row('7.f', async () => {
  let notes = [];
  let ok = true;
  const a = await newExport({ name: 'neg-nothing' });
  const ea = a.errors.join(' | ');
  await shot(page, S, '7.f-select-subject');
  const okA = a.submitted === false && ea.includes('Select at least one subject.');
  notes.push(`nothing ticked -> ${a.submitted ? 'ACCEPTED' : `"${ea}"`}`);
  const b = await newExport({ name: 'neg-param', subjects: ['tall_female_patients'] });
  await sleep(300);
  const eb = [...new Set(await visibleErrors())].join(' | ');
  await shot(page, S, '7.f-param-required');
  const stillOnForm = await page.locator('form.bulk-export-form').count();
  const okB = !!stillOnForm && eb.includes('This value is required.');
  notes.push(`query w/o min_height -> ${b.submitted ? 'ACCEPTED' : `"${eb}"${okB ? '' : ' (text differs / native browser validation?)'}`}`);
  return pf(okA && okB, notes.join('; '));
});

await row('7.m', async () => {
  const r = R.m = await runExport({ name: 'vd-csv', subjects: ['patient_demographics'], format: 'csv', header: true }, { shotName: '7.m-vd-csv' });
  if (!okFiles(r)) return pf(false, sumNote(r));
  saveSample('csv', 'vd', r.files[0].path);
  const c = await fmtCheck.csv(r.files[0]);
  const pidLine = c.data.find((l) => l.startsWith(PID + ','));
  const pidNote = pidLine ? `PID line="${pidLine}"` : 'PID not in file';
  return pf(c.header === 'id,gender,birth_date,family,city' && c.data.length === N_PAT, `header="${c.header}" dataLines=${cmp(c.data.length, N_PAT, MATRIX_PAT)} ${pidNote}`);
});

await row('7.n', async () => {
  const r = R.n = await runExport({ name: 'vd-parquet', subjects: ['patient_demographics'], format: 'parquet' }, { shotName: '7.n-vd-parquet' });
  if (!okFiles(r)) return pf(false, sumNote(r));
  saveSample('parquet', 'vd', r.files[0].path);
  const p = await fmtCheck.parquet(r.files[0]);
  const names = p.schema.map((s) => s[0]).join(',');
  const bd = p.schema.find((s) => s[0] === 'birth_date')?.[1];
  return pf(names === 'id,gender,birth_date,family,city' && p.rows === N_PAT && /string|utf8/i.test(bd || ''), `schema=${J(p.schema)} rows=${cmp(p.rows, N_PAT, MATRIX_PAT)} birth_date type=${bd}`);
});

await row('7.o', async () => {
  const r = R.o = await runExport({ name: 'query-ndjson', subjects: ['tall_female_patients'], params: { tall_female_patients: { min_height: '150' } }, format: 'ndjson' }, { shotName: '7.o-query-ndjson' });
  if (!okFiles(r)) return pf(false, sumNote(r));
  saveSample('ndjson', 'query', r.files[0].path);
  const c = await fmtCheck.ndjson(r.files[0]);
  R.o.n = c.n;
  const hasKeys = ['id', 'city', 'height'].every((k) => c.keys.includes(k));
  const eq = R.b?.data === undefined ? 'n/a (7.b gave no data)' : (R.b.data === c.n);
  return pf(hasKeys && R.b?.data === c.n, `lines=${c.n} keys=${c.keys} equals 7.b data lines (${R.b?.data})=${eq}`);
});

await row('7.p', async () => {
  const r = R.p = await runExport({ name: 'query-parquet', subjects: ['tall_female_patients'], params: { tall_female_patients: { min_height: '150' } }, format: 'parquet' }, { shotName: '7.p-query-parquet' });
  if (!okFiles(r)) return pf(false, sumNote(r));
  saveSample('parquet', 'query', r.files[0].path);
  const p = await fmtCheck.parquet(r.files[0]);
  const names = p.schema.map((s) => s[0]).join(',');
  return pf(names === 'id,city,height' && p.rows === R.o?.n, `schema=${J(p.schema)} rows=${p.rows} equals 7.o lines (${R.o?.n}); sample head=${J(p.head[0])}`);
});

await row('7.q', async () => {
  const r = R.q = await runExport({ name: 'view-ndjson', subjects: ['female_patients'], format: 'ndjson' }, { shotName: '7.q-view-ndjson' });
  if (!okFiles(r)) return pf(false, sumNote(r));
  saveSample('ndjson', 'view', r.files[0].path);
  const c = await fmtCheck.ndjson(r.files[0]);
  const noGender = c.objs.every((o) => !('gender' in o));
  return pf(c.n === N_FEM && ['id', 'birth_date', 'city'].every((k) => c.keys.includes(k)) && noGender, `lines=${cmp(c.n, N_FEM, MATRIX_FEM)} keys=${c.keys} noGenderKey=${noGender}`);
});

// ---------------------------------------------------------------- 11.5 filters
let pidRes = null;
{
  const r = await api(`/Patient/${PID}`);
  if (r.status === 200) pidRes = r.json;
}
const pidExpect = pidRes ? { gender: pidRes.gender, birth_date: pidRes.birthDate, family: pidRes.name?.[0]?.family, city: pidRes.address?.[0]?.city } : null;

await row('7.g', async () => {
  if (!pidRes) return { skip: `Patient/${PID} not on this server` };
  const search = env.T7_PID_SEARCH && pidRes.name?.[0]?.given?.some((g) => g.startsWith(env.T7_PID_SEARCH.slice(0, 4))) ? env.T7_PID_SEARCH : (pidRes.name?.[0]?.given?.[0] || 'a');
  const r = await runExport({ name: 'one-patient', subjects: ['patient_demographics'], patient: PID, patientSearch: search, format: 'ndjson' }, { shotName: '7.g-one-patient' });
  if (!okFiles(r)) return pf(false, sumNote(r));
  saveSample('ndjson', 'one-patient', r.files[0].path);
  const c = await fmtCheck.ndjson(r.files[0]);
  const o = c.objs[0] || {};
  const same = pidExpect && o.gender === pidExpect.gender && o.birth_date === pidExpect.birth_date && o.family === pidExpect.family && o.city === pidExpect.city;
  const listed = r.det.patients.some((x) => x.includes(PID));
  const matrix = o.gender === 'female' && o.birth_date === '2015-12-29' && o.family === 'Parker433' && o.city === 'Everett';
  return pf(c.n === 1 && same && listed && r.files.length === 1, `lines=${c.n} line=${J(o)} equals Patient resource=${!!same} matrixAnchorValues=${matrix} JobCardPatients=${J(r.det.patients)} comboboxSearch="${search}"`);
});

let groupReady = false;
await row('7.h', async () => {
  if (!pidRes) return { skip: `Patient/${PID} not on this server` };
  const g = await api(`/Group/${GROUP_ID}`);
  if (g.status !== 200) {
    const put = await api(`/Group/${GROUP_ID}`, { method: 'PUT', headers: { 'Content-Type': 'application/fhir+json' }, body: J({ resourceType: 'Group', id: GROUP_ID, type: 'person', actual: true, member: [{ entity: { reference: `Patient/${PID}` } }] }) });
    if (put.status >= 300) return { skip: `Group/${GROUP_ID} missing and could not be created (HTTP ${put.status})` };
    await sleep(1500);
  }
  groupReady = true;
  const r = await runExport({ name: 'one-group', subjects: ['patient_demographics'], group: GROUP_ID, format: 'json' }, { shotName: '7.h-one-group' });
  if (!okFiles(r)) return pf(false, sumNote(r));
  saveSample('json', 'one-group', r.files[0].path);
  const c = await fmtCheck.json(r.files[0]);
  const listed = r.det.groups.some((x) => x.includes(GROUP_ID));
  return pf(r.files.length === 1 && c.isArray && c.len === 1 && c.first?.id === PID && listed, `files=${r.files.length} array=${c.isArray} len=${c.len} firstId=${c.first?.id} JobCardGroups=${J(r.det.groups)}`);
});

await row('7.i', async () => {
  if (!T3_START) return { skip: 'T3_START not provided' };
  const exp = await total(`/Patient?_lastUpdated=ge${encodeURIComponent(T3_START)}`);
  const r = await runExport({ name: 'since-import', subjects: ['patient_demographics'], sincePreset: 'custom', sinceCustom: T3_START, format: 'ndjson' }, { shotName: '7.i-since-import' });
  if (!okFiles(r)) return pf(false, sumNote(r));
  const c = await fmtCheck.ndjson(r.files[0]);
  const since = r.det.facts['Since'];
  return pf(c.n === exp && !!since, `lines=${c.n} API _lastUpdated>=T3_START count=${exp} (matrix 11,704 on full corpus) Job card Since="${since}"`);
});

await row('7.j', async () => {
  if (!T3_END) return { skip: 'T3_END not provided' };
  const r = await runExport({ name: 'since-nothing', subjects: ['patient_demographics'], sincePreset: 'custom', sinceCustom: T3_END, format: 'ndjson' }, { shotName: '7.j-since-nothing' });
  if (r.card?.status !== 'complete') return pf(false, sumNote(r));
  const nOut = r.det.outputs.reduce((n, o) => n + o.files.length, 0);
  return pf(nOut === 0, `status=${r.card.label} files=${nOut} outputsCount=${r.det.facts['Output files'] || r.det.outputs.length} Since="${r.det.facts['Since']}"`);
});

await row('7.k', async () => {
  const r = await runExport({ name: 'tracked-csv-noheader', subjects: ['female_patients'], format: 'csv', header: false, tracking: 'release-check-01' }, { shotName: '7.k-tracked-csv' });
  if (!okFiles(r)) return pf(false, sumNote(r));
  saveSample('csv', 'view-noheader', r.files[0].path);
  const c = await fmtCheck.csv(r.files[0]);
  const lines = c.n;
  const hasHeader = /^id,birth_date,city/i.test(c.header);
  const fmt = r.det.facts['Format']; const trk = r.det.facts['Tracking id'];
  const man = r.manifest?.json ? J(r.manifest.json).includes('release-check-01') : false;
  return pf(!hasHeader && lines === N_FEM && /CSV · no header row/.test(fmt || '') && trk === 'release-check-01', `lines=${cmp(lines, N_FEM, MATRIX_FEM)} firstLine="${c.header}" Format="${fmt}" Tracking id="${trk}" manifest-has-clientTrackingId=${man}`);
});

await row('7.l', async () => {
  // Observed behaviour: the server reports ONE filter error per submit (patient first), so the combined form is submitted
  // first (all three bad, as in the matrix) and then each bad field is submitted alone; every message must appear and each
  // re-render must keep the other inputs.
  const jsOff = await browser.newContext({ viewport: { width: 1280, height: 800 }, javaScriptEnabled: false });
  const want = { since: 'Enter a valid FHIR instant, such as 2026-08-01T00:00:00Z.', track: 'Tracking id must be 200 characters or fewer.', pat: 'Enter only valid logical Patient IDs, separated by commas or new lines.' };
  const base = { name: 'neg-filters', subjects: ['patient_demographics'], format: 'csv' };
  const bad = { since: { sincePreset: 'custom', sinceCustom: 'yesterday' }, track: { tracking: 'x'.repeat(201) }, pat: { patient: 'not a valid id!' } };
  const notes = []; let allOk = true; let first = true;
  const attempts = [['all-three', Object.assign({}, ...Object.values(bad))], ['since', bad.since], ['track', bad.track], ['pat', bad.pat]];
  for (const [label, extra] of attempts) {
    const pg = await jsOff.newPage();
    const sub = await newExport({ ...base, ...extra }, pg, { js: false });
    const shown = [...new Set(await visibleErrors(pg))].join(' | ');
    // also messages present in the DOM but still hidden (e.g. inline hints rendered with the hidden attribute)
    const hiddenMsgs = (await pg.$$eval('.field__hint--error', (els) => els.filter((e) => e.hidden && e.textContent.trim()).map((e) => e.textContent.trim()))).join(' | ');
    await shot(pg, S, `7.l-negative-${label}`);
    const kept = {
      name: await pg.locator('input[name="name"]').inputValue(),
      subject: await pg.locator('tr[data-name="patient_demographics"] input[name="subject"]').isChecked(),
      since: await pg.locator('input[name="since_custom"]').inputValue(),
      tracking: (await pg.locator('input[name="client_tracking_id"]').inputValue()).length,
      patient: await pg.locator('textarea[name="patient"]').inputValue(),
      format: await pg.locator('input[name="format"]:checked').getAttribute('value'),
    };
    await pg.close();
    const keptOk = kept.name === 'neg-filters' && kept.subject && kept.format === 'csv' && (extra.sinceCustom === undefined || kept.since === 'yesterday') && (extra.tracking === undefined || kept.tracking === 201) && (extra.patient === undefined || kept.patient === 'not a valid id!');
    if (label === 'all-three') { notes.push(`all-three: submitted=${sub.submitted} shown="${shown}" hidden-in-DOM="${hiddenMsgs}" kept=${keptOk}`); allOk &&= !sub.submitted && keptOk; continue; }
    const okMsg = shown.includes(want[label]);
    notes.push(`${label}: ${okMsg ? 'message shown' : `message NOT shown (shown="${shown}" hidden-in-DOM="${hiddenMsgs}")`}, kept=${keptOk}, submitted=${sub.submitted}`);
    allOk &&= okMsg && keptOk && !sub.submitted;
  }
  await jsOff.close();
  return pf(allOk, notes.join('; '));
});

// ---------------------------------------------------------------- subjects-table controls
await row('7.tbl-controls', async () => {
  await go('/ui/sql/export/new');
  await page.locator('.card-head__tools--subjects').waitFor({ state: 'visible' });
  const vis = () => page.$$eval('tr[data-kind]', (rs) => rs.filter((r) => !r.hidden && r.offsetParent !== null).map((r) => r.dataset.name));
  const hint = async () => (await page.locator('#sql-export-subjects-count').innerText()).replace(/\s+/g, ' ');
  const out = [];
  const all0 = await vis();
  await page.locator('[data-subject-filter="sql-query"]').click();
  const q = await vis();
  await shot(page, S, '7.tbl-queries-filter');
  await page.locator('[data-subject-filter="all"]').click();
  const all1 = await vis();
  out.push(`Queries->${J(q)} All->${all1.length}/${all0.length}`);
  const search = page.locator('.card-head__tools--subjects input[type="search"]');
  await search.fill('female');
  const f = await vis();
  await page.locator('.table-card thead .col-check input[type="checkbox"]').check({ force: true });
  const h1 = await hint();
  await shot(page, S, '7.tbl-filter-female-select-all');
  out.push(`filter female->${J(f)} hint="${h1}"`);
  await search.fill('');
  await page.locator('tr[data-name="patient_demographics"] input[name="subject"]').check();
  await search.fill('obs');
  const h2 = await hint();
  out.push(`after tick+filter obs hint="${h2}"`);
  await search.fill('');
  const ref = await page.locator('tr[data-name="tall_female_patients"] input[name="subject"]').getAttribute('value');
  await page.locator(`tr.row--params[data-subject="${ref}"] [name="param:${ref}:min_height"]`).fill('150');
  await page.locator('input[name="name"]').fill('tbl-three');
  const okA = q.length === 1 && q[0] === 'tall_female_patients' && all1.length === all0.length;
  const okB = f.length === 2 && f.includes('female_patients') && f.includes('tall_female_patients') && /2 of 4 selected\s*·\s*1 value missing/.test(h1);
  const okC = /3 of 4 selected\s*·\s*1 value missing/.test(h2);
  await page.locator("form[action='/ui/sql/export'] button[type='submit']").click();
  await page.waitForLoadState('load');
  const w = await waitJob('tbl-three');
  const det = w.card ? await detailOf(w.card.id) : null;
  const files = det ? det.outputs.reduce((n, o) => n + o.files.length, 0) : -1;
  out.push(`submit with all three -> ${w.card?.label}, files=${files}${det?.warning ? ` warning="${det.warning}"` : ''}`);
  return pf(okA && okB && okC && w.card?.status === 'complete' && files === 3, out.join('; '));
});

await row('7.exp-as-files', async () => {
  if (!QQ) return { skip: 'query not available' };
  await go(`/ui/sql/queries?lib=${QQ}`);
  const field = page.locator('input[name="min_height"], #lib-params input[name*="min_height"]').first();
  await field.fill('150');
  await sleep(3000);
  const btn = page.getByRole('link', { name: /Export as files/ }).or(page.getByRole('button', { name: /Export as files/ })).first();
  await btn.waitFor({ state: 'visible', timeout: 15_000 });
  await shot(page, S, '7.exp-export-as-files-button');
  await btn.click();
  const msg = await confirmDialog();
  await page.waitForLoadState('load');
  await sleep(500);
  const checked = await page.locator(`input[name="subject"][value="Library/${QQ}"]`).isChecked().catch(() => false);
  await shot(page, S, '7.exp-export-as-files-landed');
  return pf(page.url().includes(`/ui/sql/export/new?subject=Library/${QQ}`) && checked, `confirm="${msg}" url=${page.url()} queryPreChecked=${checked}`);
});

// ---------------------------------------------------------------- lifecycle: Run again / Copy job id / Remove
await row('7.life', async () => {
  const notes = [];
  const cards0 = (await listCards()).filter((c) => c.name === 'vd-ndjson' && (!R.a?.card || c.id === R.a.card.id));
  if (cards0.length !== 1) return pf(false, `expected the 7.a vd-ndjson card, found ${cards0.length}`);
  const old = cards0[0];
  const preExisting = new Set((await listCards()).filter((c) => c.name === 'vd-ndjson').map((c) => c.id));
  await page.locator(`#job-${old.id} details.menu summary`).click();
  await shot(page, S, '7.life-overflow-menu');
  // Copy job id
  let copied = '';
  const copy = page.locator(`#job-${old.id} [data-copy-job-id]`);
  if (await copy.isVisible()) { await copy.click(); await sleep(300); copied = (await copy.innerText()).trim(); const clip = await page.evaluate(() => navigator.clipboard.readText().catch(() => '')); notes.push(`copy button text="${copied}" clipboard=${clip === old.id || clip === (R.a?.det?.jobId) ? 'job id' : `"${clip}"`}`); }
  else notes.push('Copy job id button not visible');
  await shot(page, S, '7.life-copied');
  // Run again
  await page.locator(`#job-${old.id} button:has-text("Run again")`).click();
  await page.waitForLoadState('load');
  const w = await waitJob('vd-ndjson', { skip: preExisting });
  const cards1 = (await listCards()).filter((c) => c.name === 'vd-ndjson' && !(preExisting.has(c.id) && c.id !== old.id));
  await shot(page, S, '7.life-run-again');
  notes.push(`run again -> ${cards1.length} cards (new ${w.card?.id} ${w.card?.label}), old ${old.id} still ${cards1.find((c) => c.id === old.id)?.label}`);
  // Remove the finished copy
  let removed = false;
  if (w.card && w.card.id !== old.id) {
    await page.locator(`#job-${w.card.id} details.menu summary`).click();
    await page.locator(`#job-${w.card.id} button:has-text("Remove from list")`).click();
    await page.waitForLoadState('load');
    const cards2 = (await listCards()).filter((c) => c.name === 'vd-ndjson' && !(preExisting.has(c.id) && c.id !== old.id));
    removed = cards2.length === 1 && cards2[0].id === old.id;
    notes.push(`after remove -> ${cards2.length} card(s)`);
  }
  return pf(copied === 'Copied' && cards1.length === 2 && w.card?.status === 'complete' && removed, notes.join('; '));
});

// ---------------------------------------------------------------- 11.6 failure and retry
await row('7.fail', async () => {
  const notes = [];
  let id = await findId('Library', 'name=broken_query');
  if (!id) {
    await fillLibrary('queries', BROKEN_JSON, 'SELECT * FROM table_that_does_not_exist');
    const prev = await previewText();
    await shot(page, S, '11.6-broken-query-lint');
    const lint = /Unknown table table_that_does_not_exist\s*—\s*line 1/.test(prev);
    const r = await saveLibrary();
    id = r.id;
    notes.push(`lint=${lint ? 'Unknown table table_that_does_not_exist — line 1' : `NOT FOUND ("${prev.replace(/\s+/g, ' ').slice(0, 160)}")`} saved=${r.saved}`);
  } else notes.push('broken_query already existed');
  const before = new Set((await listCards()).map((c) => c.id));
  const sub = await newExport({ name: 'broken', subjects: ['broken_query'], format: 'ndjson' });
  if (!sub.submitted) return pf(false, `${notes.join('; ')}; form rejected: ${J(sub.errors)}`);
  const w = await waitJob('broken', { skip: before });
  await shot(page, S, '7.fail-failed-card');
  const det = w.card ? await detailOf(w.card.id) : null;
  await shot(page, S, '7.fail-failed-detail', { full: true });
  const msgOk = det && /The export stopped on subject broken_query:/.test(det.warning);
  notes.push(`status=${w.card?.label}; warning="${det?.warning}"`);
  // Retry
  const orig = w.card;
  await go(`/ui/sql/export/${orig.id}`);
  await page.locator('button:has-text("Retry")').click();
  await page.waitForLoadState('load');
  const ids0 = new Set([...before, orig.id]);
  const w2 = await waitJob('broken', { skip: ids0 });
  const cards = (await listCards()).filter((c) => c.name === 'broken');
  await shot(page, S, '7.fail-retry-two-cards');
  const origStill = cards.find((c) => c.id === orig.id);
  notes.push(`retry -> new ${w2.card?.id} ${w2.card?.label}; original still ${origStill?.label}; cards=${cards.length}`);
  // Copy job id then Remove both
  let copied = '';
  if (cards[0]) {
    await page.locator(`#job-${cards[0].id} details.menu summary`).click();
    const copy = page.locator(`#job-${cards[0].id} [data-copy-job-id]`);
    if (await copy.isVisible()) { await copy.click(); await sleep(300); copied = (await copy.innerText()).trim(); }
    notes.push(`copy job id button="${copied}"`);
  }
  for (const c of await listCards()) {
    if (c.name !== 'broken') continue;
    await page.locator(`#job-${c.id} details.menu summary`).click();
    await page.locator(`#job-${c.id} button:has-text("Remove from list")`).click();
    await page.waitForLoadState('load');
  }
  const left = (await listCards()).filter((c) => c.name === 'broken').length;
  notes.push(`after remove both -> ${left} left`);
  // Delete broken_query
  let delMsg = null, gone = false;
  if (id) {
    await go(`/ui/sql/queries?lib=${id}`);
    await page.locator('[data-crud-delete]').click();
    delMsg = await confirmDialog();
    await sleep(1500);
    gone = (await api(`/Library/${id}`)).status === 404 || (await api(`/Library/${id}`)).status === 410;
    notes.push(`delete confirm="${delMsg}" gone=${gone}`);
  }
  return pf(w.card?.status === 'failed' && msgOk && w2.card?.status === 'failed' && origStill?.status === 'failed' && cards.length === 2 && copied === 'Copied' && left === 0 && delMsg === 'Delete "broken_query"? This cannot be undone.' && gone, notes.join('; '));
});

// ---------------------------------------------------------------- big source: JSON on a >1,000,000-row ViewDefinition
await row('7.big-json', async () => {
  const want = env.T7_BIG === undefined ? (N_OBS !== null && N_OBS >= 1_000_000) : env.T7_BIG === '1';
  if (!want) return { skip: `Observation count ${N_OBS} below 1,000,000 (set T7_BIG=1 to force); documented cap: HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD=1000000 applies to SQL Query/View dependencies` };
  const r = await runExport({ name: 'big-json', subjects: ['observation_flat'], format: 'json' }, { shotName: '7.big-json', timeout: Number(env.T7_BIG_TIMEOUT_MIN || 90) * 60_000, download: false });
  const note = [sumNote(r)];
  if (r.card?.status === 'complete') {
    note.push(`files=${r.det.outputs.map((o) => o.files.map((f) => f.label).join(',')).join(' ')}`);
    const first = r.det.outputs[0]?.files[0];
    if (first) {
      const f = await download(first.href, first.label, path.join(DL, 'big-json'));
      const fd = fs.openSync(f.path, 'r'); const head = Buffer.alloc(1); fs.readSync(fd, head, 0, 1, 0);
      const tail = Buffer.alloc(2); fs.readSync(fd, tail, 0, 2, Math.max(0, f.size - 2)); fs.closeSync(fd);
      note.push(`first file ${first.label} ${f.size} bytes starts '${head.toString()}' ends ${J(tail.toString())}`);
      fs.writeFileSync(path.join(OUTPUTS, 't7-big-json-first-file-head.txt'), fs.readFileSync(f.path).subarray(0, 4000));
      if (f.size > 200e6) fs.rmSync(f.path);
    }
  }
  // recorded as an observation: pass when the job reaches a terminal state; the note holds exactly what happened
  return pf(!!r.card && !r.timedOut, note.join(' | '));
});

// ---------------------------------------------------------------- restart behaviour
await row('7.restart', async () => {
  if (!env.T7_RESTART_CMD) return { skip: 'T7_RESTART_CMD not set (restart HFS manually while a job is In progress: its card must resolve to "Cancelled · the server no longer knows this job"; complete cards keep downloads)' };
  const before = new Set((await listCards()).map((c) => c.id));
  const sub = await newExport({ name: 'restart-me', subjects: ['observation_flat'], format: 'json' });
  if (!sub.submitted) return pf(false, `rejected: ${J(sub.errors)}`);
  const cards = await listCards();
  const c = cards.find((x) => x.name === 'restart-me' && !before.has(x.id));
  if (!c || c.status !== 'in-progress') return { skip: `job not In progress at restart time (${c?.label}); too small a dataset` };
  await shot(page, S, '7.restart-in-progress');
  const doneCard = (await listCards()).find((x) => x.name === 'vd-ndjson' && x.status === 'complete');
  execSync(env.T7_RESTART_CMD, { stdio: 'inherit', shell: '/bin/bash', timeout: 300_000 });
  const t0 = Date.now();
  while (Date.now() - t0 < 180_000) { try { const h = await fetch(BASE + '/metadata'); if (h.ok) break; } catch {} await sleep(2000); }
  await sleep(2000);
  const w = await waitJob('restart-me', { skip: before, timeout: 120_000 });
  await shot(page, S, '7.restart-after');
  const detR = w.card ? await detailOf(w.card.id) : null;
  const meta = w.card?.meta || '';
  let dlOk = 'n/a';
  if (doneCard) { const d = await detailOf(doneCard.id); const f = d.outputs[0]?.files[0]; if (f) { try { const x = await download(f.href, f.label, path.join(DL, 'after-restart')); dlOk = `${x.size} bytes`; } catch (e) { dlOk = e.message; } } }
  return pf(w.card?.status === 'cancelled' && /the server no longer knows this job/.test(`${meta} ${detR?.lede || ''} ${detR?.warning || ''}`), `card="${w.card?.label} ${meta}" detail lede="${detR?.lede}" complete-card download after restart: ${dlOk}`);
});

// ---------------------------------------------------------------- finish
await go('/ui/sql/export');
await shot(page, S, '7.final-sql-exports-list', { full: true });
const expected = ['7.a', '7.b', '7.c', '7.d', '7.e', '7.f', '7.g', '7.h', '7.i', '7.j', '7.k', '7.l', '7.m', '7.n', '7.o', '7.p', '7.q'];
for (const id of expected) if (!recorded.has(id)) rec(id, 'fail', 'row never ran');
await browser.close();
console.log('T7 done');
