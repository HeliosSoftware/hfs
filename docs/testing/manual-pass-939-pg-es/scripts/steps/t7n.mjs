// T7n -- re-run of T7 rows 7.b 7.d 7.o 7.p 7.tbl-controls 7.exp-as-files against a NARROWED observation_flat (matrix section 11).
// Why: tall_female_patients depends on observation_flat, whose full materialization exceeds HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD
// (default 1,000,000) on the full corpus. This script PUTs a new version of observation_flat with a `where` that keeps only the
// Observation codes the query uses (read from the Library SQL, e.g. 8302-2), runs the six rows (ids *-narrow), then PUTs the
// ORIGINAL definition back as a new version (7n.restore). Run: cd /home/edson/tmp/939/pw && HFS_BASE=... node steps/t7n.mjs
// Env: HFS_BASE (required), OUT, T7_JOB_TIMEOUT_MIN (default 20), T7_PY (python with pyarrow), T7_DL (default $OUT/dl-narrow),
//   T7N_CODES (comma list overriding the codes parsed from the query SQL), T7N_PROVE_UNNARROWED (1/0, default 1: before narrowing,
//   run a COUNT over the original view through $sql-run to show the limit error; costs ~1 min on the full corpus),
//   T7N_RESTORE_ONLY=1 (only PUT the saved original back, e.g. after an aborted run), EXPECT_PATIENTS / EXPECT_FEMALE (override counts).
// The original definition is saved to $OUT/outputs/t7n-observation_flat-original.json before any change.
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
const DL = env.T7_DL || path.join(OUT, 'dl-narrow');
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
const fmtCheck = {
  ndjson: async (f) => { const { n, lines } = await readLines(f.path, 1e6); const objs = lines.map((l) => JSON.parse(l)); return { n, keys: Object.keys(objs[0] || {}), objs }; },
  csv: async (f) => { const { n, lines } = await readLines(f.path, 1e6); return { n, header: lines[0], data: lines.slice(1) }; },
  json: async (f) => { const t = fs.statSync(f.path).size > 300e6 ? null : JSON.parse(fs.readFileSync(f.path, 'utf8')); return { isArray: Array.isArray(t), len: t?.length, first: t?.[0] }; },
  parquet: async (f) => pq(f.path),
};
const subjName = (fileLabel) => fileLabel;

// ================================================================ T7n specific code
const LIMIT = Number(env.T7N_LIMIT || 1_000_000);
const ORIG_FILE = path.join(OUTPUTS, 't7n-observation_flat-original.json');
const b64 = (t) => Buffer.from(t, 'utf8').toString('base64');
const unb64 = (t) => Buffer.from(t, 'base64').toString('utf8');
const FHIRJSON = { 'Content-Type': 'application/fhir+json' };
const LIB_TYPES = 'http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes';
const withSql = (lib, sql) => ({ ...lib, content: [{ contentType: 'application/sql', data: b64(sql) }] });
const stripMeta = (r) => { const { meta, ...rest } = r; return rest; };
const sameJson = (a, b) => J(a) === J(b);

/** $sql-run with an inline sql-query Library over the dependencies given: returns {status, rows|null, text}. */
async function sqlRunInline(sql, deps) {
  const lib = withSql({ resourceType: 'Library', name: 'n7n_probe', status: 'active', type: { coding: [{ system: LIB_TYPES, code: 'sql-query' }] },
    relatedArtifact: deps.map(([label, url]) => ({ type: 'depends-on', resource: url, label })) }, sql);
  const body = { resourceType: 'Parameters', parameter: [{ name: '_format', valueCode: 'json' }, { name: 'subjectResource', resource: lib }] };
  const r = await api('/$sql-run', { method: 'POST', headers: FHIRJSON, body: J(body) });
  return { status: r.status, rows: Array.isArray(r.json) ? r.json : null, text: r.text.slice(0, 300) };
}
/** $sql-run of the stored tall_female_patients query with min_height: ground truth for the export rows. */
async function sqlRunTall(minHeight) {
  const body = { resourceType: 'Parameters', parameter: [{ name: '_format', valueCode: 'json' },
    { name: 'subjectCanonical', valueString: 'http://example.org/Library/tall_female_patients' },
    { name: 'parameters', resource: { resourceType: 'Parameters', parameter: [{ name: 'min_height', valueDecimal: minHeight }] } }] };
  const r = await api('/$sql-run', { method: 'POST', headers: FHIRJSON, body: J(body), save: 't7n-sqlrun-tall-ground-truth.json' });
  return { status: r.status, rows: Array.isArray(r.json) ? r.json : null, text: r.text.slice(0, 300) };
}
async function getVd(name) {
  const r = await api(`/ViewDefinition?name=${name}&_count=5`);
  const e = r.json?.entry?.find((x) => x.resource);
  return e ? e.resource : null;
}
async function putVd(res) {
  return api(`/ViewDefinition/${res.id}`, { method: 'PUT', headers: FHIRJSON, body: J(stripMeta(res)) });
}
function sqlOf(lib) {
  const c = (lib.content || []).find((x) => /^application\/sql/.test(x.contentType || '') && x.data);
  return c ? unb64(c.data) : '';
}
function codesFromSql(sql, label) {
  const out = new Set();
  for (const m of sql.matchAll(new RegExp(`\\b${label}\\.code\\s*=\\s*'([^']+)'`, 'gi'))) out.add(m[1]);
  for (const m of sql.matchAll(new RegExp(`'([^']+)'\\s*=\\s*${label}\\.code\\b`, 'gi'))) out.add(m[1]);
  for (const m of sql.matchAll(new RegExp(`\\b${label}\\.code\\s+IN\\s*\\(([^)]*)\\)`, 'gi'))) for (const q of m[1].matchAll(/'([^']+)'/g)) out.add(q[1]);
  return [...out];
}

async function restore(note0 = '') {
  if (!fs.existsSync(ORIG_FILE)) return pf(false, 'no saved original (t7n-observation_flat-original.json); nothing to restore');
  const orig = JSON.parse(fs.readFileSync(ORIG_FILE, 'utf8').replace(/^#.*\n/, ''));
  const cur = await getVd('observation_flat');
  if (cur && sameJson(stripMeta({ ...cur, id: orig.id }), stripMeta(orig)) && !(cur.where || []).some((w) => /T7N-NARROW/.test(w.description || ''))) {
    return pf(true, `${note0}observation_flat already equals the saved original (no narrowing where present); nothing to do`);
  }
  const r = await putVd({ ...orig, id: cur?.id || orig.id });
  const back = await getVd('observation_flat');
  const same = back && sameJson(stripMeta({ ...back, id: orig.id }), stripMeta(orig));
  const vers = await api(`/ViewDefinition/${back?.id}/_history?_count=20`);
  return pf(r.status >= 200 && r.status < 300 && same,
    `${note0}PUT original observation_flat (id ${back?.id}) -> HTTP ${r.status}; stored definition equals the saved original=${same} (where=${J(back?.where ?? null)}); versions in history=${vers.json?.total ?? vers.json?.entry?.length ?? '?'}; the un-narrowed view is back for later tests`);
}

if (env.T7N_RESTORE_ONLY) {
  await row('7n.restore', () => restore('restore-only mode: '));
  await browser.close();
  process.exit(0);
}

// ---------------------------------------------------------------- prerequisites (normally created by T6/T7; created through the API when missing)
const N_PAT = Number(env.EXPECT_PATIENTS) || (await total('/Patient'));
const N_FEM = Number(env.EXPECT_FEMALE) || (await total('/Patient?gender=female'));
const N_OBS = await total('/Observation');
console.log(`expected: patients=${N_PAT} female=${N_FEM} observations=${N_OBS} limit=${LIMIT} py=${PY}`);
let QQ = null;
const prereqNotes = [];
for (const [vd, nm] of [[PD_VD, 'patient_demographics'], [OBS_VD, 'observation_flat']]) {
  if (!(await findId('ViewDefinition', `name=${nm}`))) { prereqNotes.push(`${nm} created via API (was missing)`); await post('ViewDefinition', vd); }
}
if (!(await findId('Library', 'name=female_patients'))) { prereqNotes.push('female_patients created via API (was missing)'); await post('Library', withSql(VIEW_JSON, VIEW_SQL)); }
QQ = await findId('Library', 'name=tall_female_patients');
if (!QQ) { prereqNotes.push('tall_female_patients created via API (was missing)'); QQ = await post('Library', withSql(QUERY_JSON, QUERY_SQL)); }
const qLib = (await api(`/Library/${QQ}`)).json || {};
const qSql = sqlOf(qLib);
const obsLabel = (qLib.relatedArtifact || []).find((a) => /ViewDefinition\/observation_flat/.test(a.resource || ''))?.label || 'obs';

// ---------------------------------------------------------------- 7n.setup-before: the un-narrowed dependency should hit the limit
const R = {};

let origVd = null;
let narrowed = false;
try {
  await row('7n.setup-before', async () => {
    if (env.T7N_PROVE_UNNARROWED === '0') return { skip: 'T7N_PROVE_UNNARROWED=0' };
    const cur = await getVd('observation_flat');
    if (cur?.where?.length) return { skip: `observation_flat already has a where (${J(cur.where)}); not the original, skipped` };
    const r = await sqlRunInline(`SELECT COUNT(*) AS n FROM ${obsLabel}`, [[obsLabel, 'http://example.org/ViewDefinition/observation_flat']]);
    const over = r.status === 422 || r.status === 400 ? /exceeds \d+-row limit/.test(r.text) : false;
    if (r.status === 200) return { skip: `un-narrowed view fits under the limit here (COUNT=${J(r.rows)}); nothing to narrow. Observations on server=${N_OBS}` };
    return pf(over, `un-narrowed COUNT over observation_flat (Observations on server=${N_OBS}) -> HTTP ${r.status}: ${r.text}`);
  });

  // ---------------------------------------------------------------- 7n.setup: PUT the narrowed observation_flat
  await row('7n.setup', async () => {
    const cur = await getVd('observation_flat');
    if (!cur) return pf(false, 'observation_flat not found');
    if (fs.existsSync(ORIG_FILE) && (cur.where || []).some((w) => /T7N-NARROW/.test(w.description || ''))) {
      origVd = JSON.parse(fs.readFileSync(ORIG_FILE, 'utf8').replace(/^#.*\n/, ''));
    } else {
      origVd = cur;
      fs.writeFileSync(ORIG_FILE, `# original observation_flat before T7n (${new Date().toISOString()})\n${J(cur)}`);
    }
    const codes = (env.T7N_CODES ? env.T7N_CODES.split(',') : codesFromSql(qSql, obsLabel));
    const codeNote = codes.length ? `codes ${J(codes)} ${env.T7N_CODES ? 'from T7N_CODES' : `parsed from the tall_female_patients SQL (${obsLabel}.code)`}` : '';
    const useCodes = codes.length ? codes : ['8302-2'];
    const expr = useCodes.map((c) => `code.coding.first().code = '${c}'`).join(' or ');
    // The column `code` is code.coding.first().code, so the where uses the same path: view rows == what the query would filter to.
    const next = { ...origVd, where: [...(origVd.where || []), { path: expr, description: 'T7N-NARROW: keep only the observation codes tall_female_patients uses' }] };
    const r = await putVd(next);
    if (r.status < 200 || r.status >= 300) return pf(false, `PUT narrowed observation_flat -> HTTP ${r.status} ${r.text.slice(0, 200)}`);
    narrowed = true;
    const c = await sqlRunInline(`SELECT COUNT(*) AS n FROM ${obsLabel}`, [[obsLabel, 'http://example.org/ViewDefinition/observation_flat']]);
    const n = c.rows?.[0] ? Number(Object.values(c.rows[0])[0]) : null;
    let apiN = 0; for (const code of useCodes) apiN += (await total(`/Observation?code=${encodeURIComponent(code)}`)) ?? 0;
    R.depRows = n;
    fs.writeFileSync(path.join(OUTPUTS, 't7n-setup.txt'), `PUT where=${expr}\nHTTP ${r.status}\nnarrowed dependency rows (COUNT over obs through $sql-run)=${n}\nObservation?code=${useCodes.join(',')} total=${apiN}\nObservations on server=${N_OBS}\n`);
    return pf(n !== null && n > 0 && n < LIMIT,
      `PUT narrowed observation_flat (id ${cur.id}) -> HTTP ${r.status}; where "${expr}"; ${codeNote || 'no code found in SQL, default 8302-2'}; narrowed dependency materializes ${n} rows (limit ${LIMIT}; Observations on server ${N_OBS}; API count of those codes ${apiN}${n === apiN ? ' = equal' : ' (differs: view uses the first coding only)'}); prereq: ${prereqNotes.join(', ') || 'all existed'}`);
  });

  // ---------------------------------------------------------------- ground truth for tall_female_patients (matrix numbers are for the unfiltered view)
  const gt = await sqlRunTall(150);
  const gtN = gt.rows ? gt.rows.length : null;
  const gtHeights = gt.rows ? gt.rows.map((r) => Number(r.height)) : [];
  console.log(`ground truth tall_female_patients(150): HTTP ${gt.status} rows=${gtN} ${gt.rows ? '' : gt.text}`);
  const gtNote = gtN === null ? `ground truth $sql-run unavailable (HTTP ${gt.status}: ${gt.text})` : `ground truth ($sql-run of tall_female_patients, min_height=150, narrowed view) = ${gtN} rows`;
  const gtOk = (n) => gtN === null || n === gtN;
  // sanity of the narrowing itself: the narrowed query must give the same rows as the same SQL over the full view would; checked on the API side
  // by the spot check of heights/gender in 7.b-narrow.

  // ---------------------------------------------------------------- the six rows
  await row('7.b-narrow', async () => {
    const r = R.b = await runExport({ name: 'query-csv-narrow', subjects: ['tall_female_patients'], params: { tall_female_patients: { min_height: '150' } }, format: 'csv', header: true }, { shotName: '7.b-narrow-query-csv' });
    if (!okFiles(r)) return pf(false, sumNote(r) + ' (a failure naming observation_flat with a 1000000 cap = limit still exceeded after narrowing)');
    saveSample('csv', 'query-narrow', r.files[0].path);
    const c = await fmtCheck.csv(r.files[0]);
    const hi = c.header.split(',').indexOf('height');
    const heights = c.data.map((l) => Number(csvParse(l)[hi]));
    const allGt = heights.every((h) => h > 150);
    R.b.data = c.data.length;
    const ids = c.data.map((l) => csvParse(l)[0]).slice(0, 2);
    const checks = [];
    for (const id of ids) { const p = await api(`/Patient?_id=${id}`); checks.push(`${id}=${p.json?.entry?.[0]?.resource?.gender}`); }
    const fmtLabel = r.det.facts['Format'];
    const chip = r.det.subjectChips.join(',');
    return pf(c.header === 'id,city,height' && allGt && c.data.length > 0 && checks.length > 0 && checks.every((x) => x.endsWith('=female')) && /CSV · with header row/.test(fmtLabel || '') && chip.includes(':min_height = 150') && r.files.length === 1 && gtOk(c.data.length),
      `header="${c.header}" dataRows=${c.data.length} (${gtNote}) allHeight>150=${allGt} spot=${checks.join(',')} format="${fmtLabel}" chips="${chip}" files=${r.files.length}; narrowed dependency rows=${R.depRows}`);
  });

  await row('7.d-narrow', async () => {
    const r = R.d = await runExport({ name: 'all-three-json-narrow', subjects: ['patient_demographics', 'tall_female_patients', 'female_patients'], params: { tall_female_patients: { min_height: '150' } }, format: 'json' }, { shotName: '7.d-narrow-all-three-json' });
    if (!okFiles(r)) return pf(false, sumNote(r));
    const parts = []; let allArr = true; const lens = {};
    for (const f of r.files) { const c = await fmtCheck.json(f); parts.push(`${f.label}:${c.isArray ? c.len : 'NOT-ARRAY'}`); allArr &&= c.isArray; lens[f.label] = c.len; }
    const first = r.files.find((f) => /patient_demographics/.test(f.label)) || r.files[0];
    saveSample('json', 'all-three-narrow', first.path);
    const labelsOk = r.files.length === 3 && r.files.every((f) => /^all-three-json-narrow_.+\.json$/.test(f.label));
    const lenOf = (re) => Object.entries(lens).find(([k]) => re.test(k))?.[1];
    const lp = lenOf(/patient_demographics/), lt = lenOf(/tall_female/), lf = lenOf(/json-narrow_female_patients\.json$/);
    const cnt = lp === N_PAT && lf === N_FEM && gtOk(lt);
    return pf(labelsOk && allArr && cnt, `files=${r.files.length} ${parts.join(' ')}; expected patient_demographics=${N_PAT} (API), female_patients=${N_FEM} (API), tall_female_patients=${gtN} (${gtNote}); matrix names are all-three-json_<subject>.json, here all-three-json-narrow_<subject>.json`);
  });

  await row('7.o-narrow', async () => {
    const r = R.o = await runExport({ name: 'query-ndjson-narrow', subjects: ['tall_female_patients'], params: { tall_female_patients: { min_height: '150' } }, format: 'ndjson' }, { shotName: '7.o-narrow-query-ndjson' });
    if (!okFiles(r)) return pf(false, sumNote(r));
    saveSample('ndjson', 'query-narrow', r.files[0].path);
    const c = await fmtCheck.ndjson(r.files[0]);
    R.o.n = c.n;
    const hasKeys = ['id', 'city', 'height'].every((k) => c.keys.includes(k));
    return pf(hasKeys && R.b?.data === c.n && gtOk(c.n), `lines=${c.n} keys=${c.keys} equals 7.b-narrow data lines (${R.b?.data}); ${gtNote}`);
  });

  await row('7.p-narrow', async () => {
    const r = R.p = await runExport({ name: 'query-parquet-narrow', subjects: ['tall_female_patients'], params: { tall_female_patients: { min_height: '150' } }, format: 'parquet' }, { shotName: '7.p-narrow-query-parquet' });
    if (!okFiles(r)) return pf(false, sumNote(r));
    saveSample('parquet', 'query-narrow', r.files[0].path);
    const p = await fmtCheck.parquet(r.files[0]);
    const names = p.schema.map((s) => s[0]).join(',');
    return pf(names === 'id,city,height' && p.rows === R.o?.n && gtOk(p.rows), `schema=${J(p.schema)} rows=${p.rows} equals 7.o-narrow lines (${R.o?.n}); ${gtNote}; sample head=${J(p.head[0])}`);
  });

  await row('7.tbl-controls-narrow', async () => {
    await go('/ui/sql/export/new');
    await page.locator('.card-head__tools--subjects').waitFor({ state: 'visible' });
    const vis = () => page.$$eval('tr[data-kind]', (rs) => rs.filter((r) => !r.hidden && r.offsetParent !== null).map((r) => r.dataset.name));
    const hint = async () => (await page.locator('#sql-export-subjects-count').innerText()).replace(/\s+/g, ' ');
    const out = [];
    const all0 = await vis();
    await page.locator('[data-subject-filter="sql-query"]').click();
    const q = await vis();
    await shot(page, S, '7.tbl-narrow-queries-filter');
    await page.locator('[data-subject-filter="all"]').click();
    const all1 = await vis();
    out.push(`Queries->${J(q)} All->${all1.length}/${all0.length}`);
    const search = page.locator('.card-head__tools--subjects input[type="search"]');
    await search.fill('female');
    const f = await vis();
    await page.locator('.table-card thead .col-check input[type="checkbox"]').check({ force: true });
    const h1 = await hint();
    await shot(page, S, '7.tbl-narrow-filter-female-select-all');
    out.push(`filter female->${J(f)} hint="${h1}"`);
    await search.fill('');
    await page.locator('tr[data-name="patient_demographics"] input[name="subject"]').check();
    await search.fill('obs');
    const h2 = await hint();
    out.push(`after tick+filter obs hint="${h2}"`);
    await search.fill('');
    const ref = await page.locator('tr[data-name="tall_female_patients"] input[name="subject"]').getAttribute('value');
    await page.locator(`tr.row--params[data-subject="${ref}"] [name="param:${ref}:min_height"]`).fill('150');
    await page.locator('input[name="name"]').fill('tbl-three-narrow');
    const okA = q.length === 1 && q[0] === 'tall_female_patients' && all1.length === all0.length;
    const okB = f.length === 2 && f.includes('female_patients') && f.includes('tall_female_patients') && /2 of 4 selected\s*·\s*1 value missing/.test(h1);
    const okC = /3 of 4 selected\s*·\s*1 value missing/.test(h2);
    const before = new Set((await listCards()).map((c) => c.id));
    await go('/ui/sql/export/new'); // listCards navigated away; redo the form state quickly for the final submit
    await page.locator('input[name="name"]').fill('tbl-three-narrow');
    for (const s of ['patient_demographics', 'tall_female_patients', 'female_patients']) await page.locator(`tr[data-name="${s}"] input[name="subject"]`).check();
    await page.locator(`tr.row--params[data-subject="${ref}"] [name="param:${ref}:min_height"]`).fill('150');
    await shot(page, S, '7.tbl-narrow-ready-to-submit');
    await page.locator("form[action='/ui/sql/export'] button[type='submit']").click();
    await page.waitForLoadState('load');
    const w = await waitJob('tbl-three-narrow', { skip: before });
    const det = w.card ? await detailOf(w.card.id) : null;
    if (det) await shot(page, S, '7.tbl-narrow-submitted-detail', { full: true });
    const files = det ? det.outputs.reduce((n, o) => n + o.files.length, 0) : -1;
    out.push(`submit with all three -> ${w.card?.label}, files=${files}${det?.warning ? ` warning="${det.warning}"` : ''}`);
    return pf(okA && okB && okC && w.card?.status === 'complete' && files === 3, out.join('; '));
  });

  await row('7.exp-as-files-narrow', async () => {
    if (!QQ) return { skip: 'query not available' };
    await go(`/ui/sql/queries?lib=${QQ}`);
    const field = page.locator('input[name="min_height"], #lib-params input[name*="min_height"]').first();
    await field.fill('150');
    await sleep(3000);
    const btn = page.getByRole('link', { name: /Export as files/ }).or(page.getByRole('button', { name: /Export as files/ })).first();
    await btn.waitFor({ state: 'visible', timeout: 30_000 });
    await shot(page, S, '7.exp-narrow-export-as-files-button');
    await btn.click();
    const msg = await confirmDialog();
    await page.waitForLoadState('load');
    await sleep(500);
    const checked = await page.locator(`input[name="subject"][value="Library/${QQ}"]`).isChecked().catch(() => false);
    await shot(page, S, '7.exp-narrow-export-as-files-landed');
    return pf(page.url().includes(`/ui/sql/export/new?subject=Library/${QQ}`) && checked, `confirm="${msg}" url=${page.url()} queryPreChecked=${checked}`);
  });
} finally {
  // ---------------------------------------------------------------- 7n.restore: always put the original definition back
  await row('7n.restore', () => restore(narrowed ? '' : 'narrowing never applied; '));
  for (const id of ['7n.setup', '7.b-narrow', '7.d-narrow', '7.o-narrow', '7.p-narrow', '7.tbl-controls-narrow', '7.exp-as-files-narrow']) if (!recorded.has(id)) rec(id, 'fail', 'row never ran');
  await browser.close();
  console.log('T7n done');
}
