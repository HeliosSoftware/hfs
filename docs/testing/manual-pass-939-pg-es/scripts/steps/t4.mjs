// T4 - one manual search per FHIR search type (matrix 8.1-8.4, rows 4.1-4.29).
// Env: HFS_BASE, OUT (see lib.mjs); CORPUS=full|small (default full); CORPUS_COUNTS (tsv; default full corpus file,
// cut1 file when CORPUS=small); T2_EXTRA (patients created by T2, default 1); BACKEND (default pg-es; mongodb => 4.13 na);
// LPID (Larkin patient id; else looked up by SSN); T3_START (UTC instant of 7.2, e.g. 2026-09-04T14:00:00Z; required for 4.16);
// HFS_LOG (server log file, for the 4.13 ES check); ES_URL (e.g. http://localhost:9200) + ES_PREFIX (ES evidence for 4.13).
import fs from 'fs';
import { launch, shot, record, api, BASE } from '../lib.mjs';

const S = 'T4';
const SMALL = (process.env.CORPUS || 'full') === 'small';
const BACKEND = process.env.BACKEND || 'pg-es';
const COMPOSITE = /-es$/.test(BACKEND);
const PID = '7d24f7a0-6f2e-ce3b-5568-db7b14695583';
const SSN = 'http://hl7.org/fhir/sid/us-ssn';
const SYN = 'https://github.com/synthetichealth/synthea';
const countsFile = process.env.CORPUS_COUNTS || (SMALL ? '/home/edson/hfs-t3/cut1/corpus-counts.tsv' : '/home/edson/hfs-t3/corpus/corpus-counts.tsv');
const counts = {};
for (const l of fs.readFileSync(countsFile, 'utf8').split('\n')) { const [t, n] = l.split('\t'); if (t && n) counts[t] = Number(n); }
const T2_EXTRA = Number(process.env.T2_EXTRA ?? 1);
const TOTAL_PAT = counts.Patient + T2_EXTRA;
const FH = { Accept: 'application/fhir+json' };
const today = new Date().toISOString().slice(0, 10);

// ---------- evaluation helpers ----------
const fmt = (n) => (typeof n === 'number' ? n.toLocaleString('en-US') : String(n));
const matches = (spec, v) => (typeof spec === 'number' ? v === spec : spec.ge !== undefined ? v >= spec.ge : spec.gt !== undefined ? v > spec.gt : spec.between ? v >= spec.between[0] && v <= spec.between[1] : true);
const specStr = (spec) => (typeof spec === 'number' ? fmt(spec) : spec.ge !== undefined ? `>=${fmt(spec.ge)}` : spec.gt !== undefined ? `>${fmt(spec.gt)}` : spec.between ? `${spec.between[0]}..${spec.between[1]}` : '?');
const sub = (ok, msg) => ({ ok, msg });
function finalize(id, subs) {
  const fails = subs.filter((s) => s.ok === false), nas = subs.filter((s) => s.ok === 'na'), oks = subs.filter((s) => s.ok === true);
  const note = subs.map((s) => `${s.ok === true ? 'ok' : s.ok === false ? 'FAIL' : 'na'}: ${s.msg}`).join(' | ');
  record(S, id, fails.length ? 'fail' : oks.length ? 'pass' : 'na', note);
}
const idsOf = (j) => (j?.entry || []).filter((e) => e.search?.mode !== 'include').map((e) => e.resource?.id);
const incOf = (j) => (j?.entry || []).filter((e) => e.search?.mode === 'include').length;

// ---------- UI driver ----------
const typedRequests = []; // {typed, url}
let page;
async function uiRun(typed, shotName) {
  const urlInput = page.locator('input.query-builder__url');
  const type = typed.replace(/^\//, '').split(/[?/]/)[0];
  const wait = page.waitForResponse((r) => r.request().method() === 'GET' && !r.url().includes('/ui/') && r.url().includes('/' + type) && r.url().startsWith(BASE), { timeout: 60000 }).catch(() => null);
  await urlInput.fill(`GET ${typed}`);
  await urlInput.press('Enter');
  const resp = await wait;
  if (resp) typedRequests.push({ typed, url: resp.url() });
  let meta = '';
  for (let i = 0; i < 20; i++) {
    await page.waitForTimeout(250);
    meta = ((await page.locator('#query-results-meta').textContent().catch(() => '')) || '').trim();
    const busy = await page.locator('#query-search-status').isVisible().catch(() => false);
    if (meta && !busy) break;
  }
  const err = ((await page.locator('#search-error:visible, #query-results-error:visible').first().textContent({ timeout: 300 }).catch(() => '')) || '').trim();
  const totalM = meta.match(/([\d,]+)(\+?)\s+results?/);
  const incM = meta.match(/([\d,]+)\s+included/);
  const rows = await page.locator('#query-results-body tr').count();
  const empty = rows === 1 && /No results/i.test((await page.locator('#query-results-body').textContent()) || '');
  const out = { meta, err, total: totalM ? Number(totalM[1].replace(/,/g, '')) : null, partial: !!(totalM && totalM[2]), inc: incM ? Number(incM[1].replace(/,/g, '')) : 0, rows: empty ? 0 : rows, status: resp?.status() ?? null };
  if (shotName) await shot(page, S, shotName);
  return out;
}
async function apiRun(typed, save) {
  const u = typed.includes('_total=') || typed.includes('_summary=count') ? typed : typed + (typed.includes('?') ? '&' : '?') + '_total=accurate';
  const r = await api(u, { headers: FH, save });
  return { status: r.status, total: r.json?.total ?? null, ids: idsOf(r.json), inc: incOf(r.json), json: r.json, text: r.text };
}
// run a query in UI + API; shotName e.g. '4.1a-family'
async function both(typed, shotName, save) {
  const ui = await uiRun(typed, shotName);
  const ap = await apiRun(typed, save);
  return { typed, ui, api: ap };
}
// one expectation: total (spec), inc (number), corpus (bool: value depends on the full corpus), has (ids that must be among the rows)
function expect(r, e, label = r.typed) {
  const parts = [];
  let ok = true;
  const { ui, api: ap } = r;
  const obs = `ui ${ui.total === null ? `"${ui.meta}"${ui.err ? ' err=' + ui.err : ''}` : fmt(ui.total) + (ui.inc ? ` +${ui.inc}inc` : '')}, api ${ap.total === null ? 'status ' + ap.status : fmt(ap.total) + (ap.inc ? ` +${ap.inc}inc` : '')}`;
  if (ui.total !== ap.total) { ok = false; parts.push('UI/API total disagree'); }
  if (e.total !== undefined && !matches(e.total, ap.total ?? -1)) { ok = e.corpus && SMALL ? 'na' : false; parts.push(`expected ${specStr(e.total)}${ok === 'na' ? ' (small corpus)' : ''}`); }
  if (e.inc !== undefined) {
    if (ui.inc !== e.inc || ap.inc !== e.inc) { if (ok === true) ok = e.corpus && SMALL ? 'na' : false; parts.push(`expected ${e.inc} included`); }
  } else if (e.noInc && (ui.inc || ap.inc)) { ok = false; parts.push('expected no included count'); }
  for (const id of e.has || []) if (!ap.ids.includes(id)) { ok = false; parts.push(`row ${id} missing`); }
  if (e.rows !== undefined && ui.rows !== e.rows) { ok = false; parts.push(`expected ${e.rows} UI rows, got ${ui.rows}`); }
  return sub(ok, `${label} => ${obs}${parts.length ? ' [' + parts.join('; ') + ']' : ''}`);
}
async function items(id, list, tag) {
  const subs = [], res = [];
  let i = 0;
  for (const it of list) {
    const r = await both(it.q, `${id}${String.fromCharCode(97 + i)}-${tag}`, `t4-${id}${String.fromCharCode(97 + i)}.json`);
    res.push(r); subs.push(expect(r, it)); i++;
  }
  return { subs, res };
}
async function tryRow(id, fn) {
  try { await fn(); } catch (e) { record(S, id, 'fail', `script error: ${String(e.message || e).split('\n')[0]}`); }
}
async function openFirstRow(name) {
  await page.locator('#query-results-body tr').first().locator('a, td').first().click();
  await page.locator('#resource-modal:not([hidden])').waitFor({ timeout: 10000 });
  await page.waitForTimeout(800);
  // CodeMirror renders only the visible lines: scroll through the editor and accumulate the text
  let txt = '';
  const sc = page.locator('#resource-modal .cm-scroller').first();
  if (await sc.count()) {
    const seen = new Set();
    for (let i = 0; i < 60; i++) {
      const t = await page.locator('#resource-modal .cm-content').first().innerText().catch(() => '');
      for (const l of t.split('\n')) if (!seen.has(l)) { seen.add(l); txt += l + '\n'; }
      const atEnd = await sc.evaluate((e) => { e.scrollTop += Math.max(200, e.clientHeight - 60); return e.scrollTop + e.clientHeight >= e.scrollHeight - 2; });
      await page.waitForTimeout(120);
      if (atEnd) { const t2 = await page.locator('#resource-modal .cm-content').first().innerText().catch(() => ''); for (const l of t2.split('\n')) if (!seen.has(l)) { seen.add(l); txt += l + '\n'; } break; }
    }
    await sc.evaluate((e) => { e.scrollTop = 0; });
    await page.waitForTimeout(200);
  } else txt = await page.locator('#resource-modal textarea').first().innerText().catch(() => '');
  if (name) await shot(page, S, name);
  await page.keyboard.press('Escape');
  await page.waitForTimeout(300);
  if (await page.locator('#resource-modal:not([hidden])').count()) { await page.locator('#resource-modal [data-close], #resource-modal button[aria-label]').first().click({ timeout: 2000 }).catch(() => {}); await page.waitForTimeout(300); }
  return txt;
}
async function allPages(typed, max = 400) { // collect resources via the API (follow next links)
  const out = []; let r = await api(typed + (typed.includes('_count') ? '' : (typed.includes('?') ? '&' : '?') + '_count=100'), { headers: FH });
  for (let i = 0; i < 20 && r.json; i++) {
    for (const e of r.json.entry || []) if (e.search?.mode !== 'include') out.push(e.resource);
    const next = (r.json.link || []).find((l) => l.relation === 'next')?.url;
    if (!next || out.length >= max) break;
    r = await api(next, { headers: FH });
  }
  return out;
}

// ---------- ES evidence ----------
async function esProbe() {
  const p = { logLen: null, esQ: null };
  try { if (process.env.HFS_LOG) p.logLen = fs.statSync(process.env.HFS_LOG).size; } catch {}
  try {
    if (process.env.ES_URL) {
      const r = await fetch(`${process.env.ES_URL}/${process.env.ES_PREFIX || 'hfs'}_*/_stats/search`);
      p.esQ = (await r.json())._all?.total?.search?.query_total ?? null;
    }
  } catch {}
  return p;
}
async function esDiff(before) {
  const after = await esProbe();
  const out = { logHit: null, logLines: [], esDelta: null };
  if (before.logLen !== null && after.logLen !== null) {
    const buf = fs.readFileSync(process.env.HFS_LOG).subarray(before.logLen).toString();
    out.logLines = buf.split('\n').filter((l) => /elasticsearch|Routing query/i.test(l)).slice(0, 4);
    out.logHit = out.logLines.some((l) => /elasticsearch/i.test(l));
  }
  if (before.esQ !== null && after.esQ !== null) out.esDelta = after.esQ - before.esQ;
  return out;
}

// =====================================================================
const { browser, page: pg } = await launch();
page = pg;
page.setDefaultTimeout(15000);
const consoleErrors = [];
page.on('pageerror', (e) => consoleErrors.push(String(e)));

// ---------- 8.1 fixtures (Resource Editor, raw JSON) ----------
async function setJson(json) {
  const cm = page.locator('.cm-content').first();
  if (await cm.count()) { await cm.click(); await page.keyboard.press('Control+A'); await page.keyboard.insertText(json); }
  else await page.locator('#editor-source').fill(json);
  await page.waitForTimeout(1200);
}
async function fixture(id, type, json, apiPath, check) {
  try {
    await page.goto(`${BASE}/ui/editor?type=${type}`);
    await page.locator('#editor-body').waitFor();
    await page.waitForTimeout(1200);
    await setJson(json);
    const chip = ((await page.getByText('No issues.').first().textContent({ timeout: 4000 }).catch(() => '')) || '').trim();
    await shot(page, S, `${id}-editor-${type}-json`);
    await page.locator('#editor-save').click();
    await page.waitForTimeout(1500);
    // a rerun finds the id already present: confirm the "already exists" dialog (not shown on a fresh server)
    const again = page.getByRole('button', { name: 'Save new version' });
    if (await again.isVisible().catch(() => false)) { await shot(page, S, `${id}-editor-${type}-exists-dialog`); await again.click(); }
    await page.waitForTimeout(1500);
    // the matrix says "save as PUT because the document carries an id"; an existing id may ask for confirmation
    const pill = await page.locator('.tag--unsaved:visible').count();
    const status = ((await page.locator('#editor-status').textContent().catch(() => '')) || '').trim();
    await shot(page, S, `${id}-editor-${type}-saved`);
    const g = await api(apiPath, { headers: FH, save: `t4-${id}.json` });
    const subs = [
      sub(chip === 'No issues.', `chip "${chip}" (expected "No issues.")`),
      sub(pill === 0, `Unsaved changes pill count ${pill} (expected 0)`),
      sub(!/error|invalid|fail/i.test(status), `status line "${status}"`),
      sub(g.status === 200 && check(g.json), `GET ${apiPath} -> ${g.status}`),
    ];
    finalize(id, subs);
  } catch (e) { record(S, id, 'fail', `script error: ${String(e.message || e).split('\n')[0]}`); }
}
await fixture('8.1a', 'RiskAssessment',
  `{"resourceType":"RiskAssessment","id":"manual-risk","status":"final","subject":{"reference":"Patient/${PID}"},"prediction":[{"probabilityDecimal":0.8}]}`,
  '/RiskAssessment/manual-risk', (j) => j?.prediction?.[0]?.probabilityDecimal === 0.8);
await fixture('8.1b', 'ValueSet',
  '{"resourceType":"ValueSet","id":"manual-test-vs","status":"active","url":"http://example.org/fhir/ValueSet/manual-test","name":"ManualTest"}',
  '/ValueSet/manual-test-vs', (j) => j?.url === 'http://example.org/fhir/ValueSet/manual-test');

// resolve LPID
let LPID = process.env.LPID || '';
if (!LPID) { const r = await api(`/Patient?identifier=${SSN}|999-19-2626`, { headers: FH }); LPID = r.json?.entry?.[0]?.resource?.id || ''; }
console.log('LPID =', LPID || '(not found)');
const needL = (id, fn) => (LPID ? fn() : (record(S, id, 'na', 'LPID not found (T2 patient absent in this server)'), null));

await page.goto(`${BASE}/ui/resources`);
await page.locator('input.query-builder__url').waitFor();
await page.waitForTimeout(1000);

// ---------- 4.1 string ----------
await tryRow('4.1', async () => {
  const { subs, res } = await items('4.1', [
    { q: '/Patient?family=Parker433', total: 30, corpus: true },
    { q: '/Patient?family:exact=Parker433', total: 30, corpus: true },
    { q: '/Patient?family:contains=arker43', total: { ge: 30 }, corpus: true },
    { q: '/Patient?name=cari853', total: 3, corpus: true, has: [PID] },
    { q: '/Patient?name=Cari853', total: 3, corpus: true, has: [PID] },
    { q: '/Patient?given=Cari853', total: 3, corpus: true, has: [PID] },
    { q: '/Patient?name:exact=Cari853', total: 3, corpus: true, has: [PID] },
  ], 'string');
  const t = res.slice(3).map((r) => r.api.total);
  subs.push(sub(t.every((x) => x === t[0]), `name=cari853/Cari853/given/name:exact equal totals: ${t.join(',')}`));
  finalize('4.1', subs);
  // POST search equivalent of the first query
  const p = await api('/Patient/_search', { method: 'POST', headers: { ...FH, 'Content-Type': 'application/x-www-form-urlencoded' }, body: 'family=Parker433&_total=accurate', save: 't4-4.1-post.json' });
  const g = res[0].api.total;
  record(S, '4.1-post', p.status === 200 && p.json?.total === g ? 'pass' : 'fail', `POST /Patient/_search family=Parker433 -> ${p.status} total ${p.json?.total} vs GET ${g}`);
});

// ---------- 4.2 token ----------
await tryRow('4.2', async () => {
  const { subs, res } = await items('4.2', [
    { q: '/Patient?gender=female', total: 5814, corpus: true },
    { q: '/Patient?gender:not=female', total: 5891, corpus: true },
    { q: `/Patient?identifier=${SSN}|999-33-3920`, total: 1, has: [PID] },
    { q: '/Observation?code=http://loinc.org|8302-2', total: { gt: 175000 }, corpus: true },
    { q: '/Observation?code=8302-2', total: { gt: 175000 }, corpus: true },
  ], 'token');
  subs.push(sub(res[0].api.total + res[1].api.total === TOTAL_PAT, `female + not-female = ${res[0].api.total}+${res[1].api.total} (expected total patients ${fmt(TOTAL_PAT)})`));
  subs.push(sub(res[3].api.total === res[4].api.total, `system|code vs code-only totals ${res[3].api.total} / ${res[4].api.total} (expected identical)`));
  finalize('4.2', subs);
});

// ---------- 4.3 date ----------
await tryRow('4.3', async () => {
  const { subs, res } = await items('4.3', [
    { q: '/Patient?birthdate=ge1980-01-01&birthdate=lt1990-01-01', total: 1268, corpus: true },
    { q: `/Encounter?patient=${PID}&date=ge2016`, total: { between: [1, 24] } },
    { q: `/Patient?_lastUpdated=ge${today}`, total: TOTAL_PAT },
  ], 'date');
  const enc = await allPages(`/Encounter?patient=${PID}&date=ge2016`);
  const bad = enc.filter((e) => !(e.period?.start >= '2016'));
  subs.push(sub(enc.length > 0 && bad.length === 0, `${enc.length} encounters, every period.start >= 2016 (violations ${bad.length})`));
  finalize('4.3', subs);
});

// ---------- 4.4 number ----------
await tryRow('4.4', async () => {
  const { subs } = await items('4.4', [
    { q: '/RiskAssessment?probability=gt0.5', total: 1, has: ['manual-risk'] },
    { q: '/RiskAssessment?probability=lt0.5', total: 0 },
    { q: '/RiskAssessment?probability=ap0.8', total: 1, has: ['manual-risk'] },
  ], 'number');
  finalize('4.4', subs);
});

// ---------- 4.5 quantity ----------
let heightGt150 = null;
await tryRow('4.5', async () => {
  const { subs, res } = await items('4.5', [
    { q: '/Observation?code=8302-2&value-quantity=gt150', total: { gt: 0 } },
    { q: '/Observation?code=8302-2&value-quantity=gt150||cm', total: { gt: 0 } },
    { q: '/Observation?code=8302-2&value-quantity=lt50|http://unitsofmeasure.org|cm', total: { ge: 0 }, corpus: true },
  ], 'quantity');
  heightGt150 = res[0].api.total;
  subs.push(sub(res[0].api.total === res[1].api.total, `gt150 vs gt150||cm totals ${res[0].api.total}/${res[1].api.total} (expected equal)`));
  subs.push(sub(res[2].api.total < res[0].api.total, `lt50 subset ${res[2].api.total} < ${res[0].api.total}`));
  const obs = await allPages('/Observation?code=8302-2&value-quantity=gt150', 100);
  const low = obs.filter((o) => !(o.valueQuantity?.value > 150));
  subs.push(sub(obs.length > 0 && low.length === 0, `first ${obs.length} rows all valueQuantity.value > 150 (violations ${low.length})`));
  await uiRun('/Observation?code=8302-2&value-quantity=gt150');
  const txt = await openFirstRow('4.5d-row-open');
  const m = txt.match(/"value":\s*([\d.]+)/);
  subs.push(sub(!!m && Number(m[1]) > 150, `opened first row (Edit Resource modal) valueQuantity.value ${m ? m[1] : 'not found'}`));
  finalize('4.5', subs);
});

// ---------- 4.6 reference ----------
await tryRow('4.6', async () => {
  const { subs, res } = await items('4.6', [
    { q: `/Observation?subject=Patient/${PID}`, total: 165 },
    { q: `/Condition?patient=${PID}`, total: 15 },
    { q: `/Encounter?subject=${PID}&_include=Encounter:subject`, total: 24, inc: 1 },
  ], 'reference');
  subs.push(sub(res[2].api.inc === 1, `API include entries ${res[2].api.inc} (expected 1)`));
  finalize('4.6', subs);
});

// ---------- 4.7 uri ----------
await tryRow('4.7', async () => {
  const { subs } = await items('4.7', [
    { q: '/ValueSet?url=http://example.org/fhir/ValueSet/manual-test', total: 1, has: ['manual-test-vs'] },
    { q: '/ValueSet?url:below=http://example.org/fhir', total: { ge: 1 }, has: ['manual-test-vs'] },
  ], 'uri');
  finalize('4.7', subs);
});

// ---------- 4.8 composite ----------
await tryRow('4.8', async () => {
  const es = await esProbe();
  const { subs } = await items('4.8', [
    { q: '/Observation?code-value-quantity=http://loinc.org|8302-2$gt150', total: { gt: 0 } },
  ], 'composite');
  const first = await apiRun('/Observation?code-value-quantity=http://loinc.org|8302-2$gt150');
  subs.push(sub(heightGt150 !== null && first.total === heightGt150, `composite total ${first.total} equals 4.5 first count ${heightGt150}`));
  const obs = await allPages('/Observation?code-value-quantity=http://loinc.org|8302-2$gt150', 100);
  const bad = obs.filter((o) => !(o.code?.coding?.some((c) => c.code === '8302-2') && o.valueQuantity?.value > 150));
  subs.push(sub(obs.length > 0 && bad.length === 0, `${obs.length} rows all Body Height (8302-2) with value > 150 (violations ${bad.length})`));
  const d = await esDiff(es);
  subs.push(sub(true, `ES evidence (informational): query_total delta ${d.esDelta}, log lines ${d.logLines.length}`));
  finalize('4.8', subs);
});

// ---------- 4.9 _id ----------
await tryRow('4.9', async () => {
  const { subs } = await items('4.9', [
    { q: `/Patient?_id=${PID}`, total: 1, has: [PID] },
    LPID ? { q: `/Patient?_id=${PID},${LPID}`, total: 2, has: [PID, LPID] } : { q: `/Patient?_id=${PID}`, total: 1 },
  ], 'id');
  if (!LPID) subs[1] = sub('na', 'LPID not available; second query skipped');
  finalize('4.9', subs);
});

// ---------- 4.10 chained ----------
await tryRow('4.10', async () => {
  const { subs, res } = await items('4.10', [
    { q: `/Observation?subject.identifier=${SSN}|999-33-3920`, total: 165 },
    { q: '/Observation?subject:Patient.family=Parker433&_count=5', total: { gt: 165 }, corpus: true, rows: 5 },
  ], 'chained');
  const pk = (await allPages('/Patient?family=Parker433')).map((p) => 'Patient/' + p.id);
  const rows = res[1].api.json?.entry?.map((e) => e.resource?.subject?.reference) || [];
  subs.push(sub(rows.length > 0 && rows.every((x) => pk.includes(x)), `${rows.length} rows subject.reference in the ${pk.length} family=Parker433 patients`));
  await openFirstRow('4.10c-row-open');
  finalize('4.10', subs);
});

// ---------- 4.11 reverse chained ----------
await tryRow('4.11', async () => {
  const { subs, res } = await items('4.11', [
    { q: '/Patient?_has:Observation:patient:code=http://loinc.org|8302-2&_count=5', total: { gt: 0 } },
  ], 'has');
  const pid = res[0].api.ids[0];
  if (pid) { const r = await both(`/Observation?patient=${pid}&code=8302-2`, '4.11b-follow-up'); subs.push(expect(r, { total: { gt: 0 } })); }
  else subs.push(sub(false, 'no row to follow'));
  finalize('4.11', subs);
});

// ---------- 4.12 _revinclude / _sort / paging ----------
await tryRow('4.12', async () => {
  const { subs, res } = await items('4.12', [
    { q: `/Patient?_id=${PID}&_revinclude=Condition:patient`, total: 1, inc: 15 },
    { q: `/Observation?patient=${PID}&_sort=-date&_count=5`, total: 165, rows: 5 },
  ], 'revinclude-sort');
  const eff = (res[1].api.json?.entry || []).map((e) => e.resource?.effectiveDateTime || e.resource?.effectivePeriod?.start);
  subs.push(sub(eff.length === 5 && eff.every((v, i) => i === 0 || eff[i - 1] >= v), `effective dates descending: ${eff.join(' > ')}`));
  // Sort dropdown
  const sortOpts = [['-_lastUpdated', 'recent'], ['_lastUpdated', 'oldest'], ['_id', 'id']];
  for (const [val, nm] of sortOpts) {
    try {
      const waitR = page.waitForResponse((r) => r.request().method() === 'GET' && !r.url().includes('/ui/') && r.url().includes('/Observation') && r.url().includes('_sort='), { timeout: 20000 }).catch(() => null);
      await page.locator('#query-results-sort').selectOption(val, { force: true });
      const rr = await waitR; await page.waitForTimeout(800);
      const qv = await page.locator('input.query-builder__url').inputValue();
      subs.push(sub(qv.includes(`_sort=${val}`) && !!rr && rr.url().includes(encodeURIComponent(val).replace(/%2C/g, ',')) || qv.includes(`_sort=${val}`), `Sort dropdown ${nm}: QUERY box "${qv}"`));
      await shot(page, S, `4.12c-sort-${nm}`);
    } catch (e) { subs.push(sub(false, `Sort dropdown ${nm}: ${String(e.message).split('\n')[0]}`)); }
  }
  // paging
  const pr = await both('/Patient?_count=20&_total=accurate', '4.12d-paging-page1', 't4-4.12-page1.json');
  subs.push(expect(pr, { total: TOTAL_PAT, rows: 20 }, 'paging page 1'));
  const nextVis = await page.locator('#query-results-next').isVisible();
  subs.push(sub(nextVis || TOTAL_PAT <= 20, `Next visible: ${nextVis}`));
  if (nextVis) {
    const w = page.waitForResponse((r) => r.request().method() === 'GET' && !r.url().includes('/ui/') && r.url().includes('/Patient'), { timeout: 20000 }).catch(() => null);
    await page.locator('#query-results-next').click(); await w; await page.waitForTimeout(1000);
    const meta = ((await page.locator('#query-results-meta').textContent()) || '').trim();
    const tot = Number((meta.match(/([\d,]+)\+?\s+results?/) || [])[1]?.replace(/,/g, ''));
    const prevVis = await page.locator('#query-results-prev').isVisible();
    await shot(page, S, '4.12e-paging-page2');
    subs.push(sub(tot === TOTAL_PAT && prevVis, `after Next: "${meta}", Previous visible ${prevVis} (expected total ${fmt(TOTAL_PAT)} unchanged)`));
  }
  finalize('4.12', subs);
});

// ---------- 4.13 _content ----------
await tryRow('4.13', async () => {
  if (BACKEND === 'mongodb') {
    const r = await both('/Patient?_content=Everett', '4.13a-content');
    record(S, '4.13', 'na', `mongodb: standalone MongoDB has no full-text search; API status ${r.api.status}: ${r.api.text.slice(0, 160)}; UI "${r.ui.meta}${r.ui.err}"`);
    return;
  }
  const es = await esProbe();
  const { subs } = await items('4.13', [{ q: '/Patient?_content=Everett', total: { ge: 83 }, corpus: true }], 'content');
  const d = await esDiff(es);
  if (COMPOSITE) {
    if (d.logHit === null && d.esDelta === null) subs.push(sub('na', 'no HFS_LOG / ES_URL given: cannot confirm that Elasticsearch served the search'));
    else subs.push(sub(d.logHit === true || (d.esDelta ?? 0) > 0, `ES served it? log elasticsearch lines: ${d.logLines.length ? JSON.stringify(d.logLines.map((l) => l.slice(0, 200))) : 'none'}; ES search query_total delta ${d.esDelta}`));
  } else subs.push(sub('na', `backend ${BACKEND} is not a composite: ES log check n/a`));
  finalize('4.13', subs);
});

// ---------- 4.14 visual builder + saved/recent ----------
await tryRow('4.14', async () => {
  const subs = [];
  await page.goto(`${BASE}/ui/resources`);
  await page.locator('input.query-builder__url').waitFor();
  await page.waitForTimeout(800);
  // builder mode (only when a natural-language key is configured does a mode switch exist)
  const modeBtn = page.locator('[data-mode-btn="builder"]');
  if (await modeBtn.count()) await modeBtn.click();
  // choose the Patient type in the rail
  await page.locator('#type-rail-list').getByText('Patient', { exact: true }).first().click();
  await page.waitForTimeout(1200);
  await page.locator('#builder-sections').waitFor({ state: 'visible', timeout: 8000 });
  // remove the starting _summary=true result row
  const ctrlRows = page.locator('#builder-controls > *');
  for (let i = 0; i < 3 && (await ctrlRows.count()) > 0; i++) {
    const t = await ctrlRows.first().innerText().catch(() => '');
    const x = ctrlRows.first().locator('button').last();
    if (/_summary/.test(t) || (await ctrlRows.first().locator('input').first().inputValue().catch(() => '')) === '_summary') await x.click(); else break;
    await page.waitForTimeout(300);
  }
  await shot(page, S, '4.14a-builder-start');
  // + Add condition: family is Parker433
  await page.locator('[data-add="condition"]').click();
  let row = page.locator('#builder-conditions > *').last();
  const fillRow = async (rowLoc, key, value) => {
    const inputs = rowLoc.locator('input');
    await inputs.first().fill(key); await inputs.first().press('Tab').catch(() => {});
    await inputs.last().fill(value);
    await page.waitForTimeout(300);
  };
  await fillRow(row, 'family', 'Parker433');
  await page.locator('[data-add="condition"]').click();
  row = page.locator('#builder-conditions > *').last();
  await fillRow(row, 'birthdate', '2010-01-01');
  // comparator ge
  const sel = row.locator('select');
  const selN = await sel.count();
  for (let i = 0; i < selN; i++) { const opts = await sel.nth(i).locator('option').allInnerTexts(); if (opts.some((o) => o.trim() === 'ge')) { await sel.nth(i).selectOption({ label: 'ge' }, { force: true }); break; } }
  await page.locator('[data-add="control"]').click();
  const crow = page.locator('#builder-controls > *').last();
  await crow.locator('select.builder-row__key').selectOption('_sort', { force: true });
  await crow.locator('input').last().fill('birthdate');
  await page.waitForTimeout(500);
  const qv = await page.locator('input.query-builder__url').inputValue();
  await shot(page, S, '4.14b-builder-conditions');
  subs.push(sub(qv === 'GET /Patient?family=Parker433&birthdate=ge2010-01-01&_sort=birthdate', `QUERY box reads "${qv}"`));
  const r = await uiRun('/Patient?family=Parker433&birthdate=ge2010-01-01&_sort=birthdate', '4.14c-builder-run');
  const ap = await apiRun('/Patient?family=Parker433&birthdate=ge2010-01-01&_sort=birthdate');
  const bds = (ap.json?.entry || []).map((e) => e.resource.birthDate);
  subs.push(sub(r.total === ap.total && r.total >= 1 && bds.every((b, i) => i === 0 || bds[i - 1] <= b), `Run: ui ${r.total}, api ${ap.total} Parker433 children, birth dates ascending: ${bds.join(',')}`));
  // Recent disclosure
  await page.locator('summary.query-recent__toggle').click();
  await page.waitForTimeout(400);
  const recent1 = ((await page.locator('#recent-searches').innerText()) || '').trim();
  await shot(page, S, '4.14d-recent-open');
  subs.push(sub(/Parker433/.test(recent1), `Recent disclosure lists the query: "${recent1.replace(/\s+/g, ' ').slice(0, 160)}"`));
  await page.locator('summary.query-recent__toggle').click().catch(() => {});
  // run broad, run parker433, load broad from Recent without Run
  await uiRun('/Patient?_summary=true');
  const r2 = await uiRun('/Patient?family=Parker433', '4.14e-parker433');
  await page.locator('summary.query-recent__toggle').click();
  await page.waitForTimeout(400);
  const broad = page.locator('#recent-searches').getByText(/_summary=true/).first();
  await broad.click();
  await page.waitForTimeout(800);
  const prevTag = await page.locator('#query-results-previous').isVisible();
  const metaNow = ((await page.locator('#query-results-meta').textContent()) || '').trim();
  const qv2 = await page.locator('input.query-builder__url').inputValue();
  await shot(page, S, '4.14f-recent-broad-loaded');
  subs.push(sub(prevTag && metaNow === r2.meta && /_summary=true/.test(qv2), `loading broad query from Recent: QUERY "${qv2}", Previous results visible ${prevTag}, table/count retained "${metaNow}" (Parker433 meta "${r2.meta}")`));
  await page.locator('button[data-intent="run"]').click();
  await page.waitForTimeout(2500);
  const prevTag2 = await page.locator('#query-results-previous').isVisible();
  const metaAfter = ((await page.locator('#query-results-meta').textContent()) || '').trim();
  await shot(page, S, '4.14g-recent-broad-run');
  subs.push(sub(!prevTag2 && metaAfter !== metaNow, `Run after Recent load: label hidden ${!prevTag2}, meta "${metaAfter}" (was "${metaNow}")`));
  // loading the query already represented by the results keeps the label hidden
  await page.locator('summary.query-recent__toggle').click().catch(() => {});
  await page.waitForTimeout(300);
  const again = page.locator('#recent-searches').getByText(/_summary=true/).first();
  if (await again.count()) { await again.click().catch(() => {}); await page.waitForTimeout(600); }
  const prevTag3 = await page.locator('#query-results-previous').isVisible();
  subs.push(sub(!prevTag3, `loading the query already shown keeps label hidden (visible=${prevTag3})`));
  // Saved named query from user settings: needs a named query in settings; not created by this script
  const saved = await page.locator('#recent-searches').innerText().catch(() => '');
  subs.push(sub('na', `Saved (user-settings named query) loading not exercised unless a named query exists; Recent text sample "${saved.replace(/\s+/g, ' ').slice(0, 80)}"`));
  finalize('4.14', subs);
});

// ---------- 8.3 (T2 data) ----------
await page.goto(`${BASE}/ui/resources`);
await page.locator('input.query-builder__url').waitFor();
await page.waitForTimeout(800);

await tryRow('4.15', () => needL('4.15', async () => {
  const { subs } = await items('4.15', [
    { q: `/Patient?identifier=${SSN}|999-19-2626`, total: 1, has: [LPID] },
    { q: '/Patient?address-city=Millis', total: 19, corpus: true },
    { q: '/Patient?address-city:exact=Millis', total: 13, corpus: true },
    { q: '/Patient?family=Larkin917&given=Nicky270&gender=female', total: { ge: 1 }, has: [LPID] },
  ], 'token-string');
  finalize('4.15', subs);
}));

await tryRow('4.16', () => needL('4.16', async () => {
  const t3 = process.env.T3_START;
  if (!t3) { record(S, '4.16', 'na', 'T3_START env (instant of 7.2, UTC) not given'); return; }
  const { subs } = await items('4.16', [
    { q: `/Patient?_lastUpdated=lt${t3}`, total: 1, has: [LPID] },
    { q: `/Patient?_lastUpdated=ge${t3}`, total: counts.Patient },
  ], 'lastupdated');
  finalize('4.16', subs);
}));

await tryRow('4.17', () => needL('4.17', async () => {
  const { subs } = await items('4.17', [
    { q: `/Encounter?patient=${LPID}&class=EMER`, total: 5 },
    { q: `/Encounter?patient=${LPID}&class=IMP`, total: 1 },
    { q: `/Encounter?patient=${LPID}&date=ge2020`, total: 38 },
    { q: `/Encounter?patient=${LPID}&type=http://snomed.info/sct|424619006`, total: 17 },
  ], 'token-date');
  finalize('4.17', subs);
}));

await tryRow('4.18', () => needL('4.18', async () => {
  const { subs } = await items('4.18', [
    { q: `/Encounter?patient=${LPID}&_include=Encounter:service-provider`, total: 49, inc: 4 },
    { q: `/Encounter?patient=${LPID}&_include=Encounter:participant`, total: 49, inc: 4 },
  ], 'include');
  const rs = await allPages(`/Encounter?patient=${LPID}&_include=Encounter:service-provider&_count=100`);
  subs.push(sub(rs.length > 0 && rs.every((e) => /^Organization\//.test(e.serviceProvider?.reference || '')), `all ${rs.length} serviceProvider.reference start with Organization/ (e.g. ${rs[0]?.serviceProvider?.reference})`));
  finalize('4.18', subs);
}));

await tryRow('4.19', () => needL('4.19', async () => {
  const { subs } = await items('4.19', [
    { q: `/Encounter?patient=${LPID}&service-provider.name=ENCOMPASS`, total: 38 },
    { q: `/Encounter?patient=${LPID}&participant.identifier=http://hl7.org/fhir/sid/us-npi|9999989798`, total: 38 },
  ], 'chained');
  finalize('4.19', subs);
}));

await tryRow('4.20', async () => {
  const { subs } = await items('4.20', [
    { q: '/Organization?name=TIMOTHY DANIELS HOUSE', total: 2, corpus: true },
    { q: '/Organization?address-city=HOLLISTON', total: 2, corpus: true },
    { q: '/Practitioner?identifier=http://hl7.org/fhir/sid/us-npi|9999888693', total: 2, corpus: true },
    { q: '/Practitioner?family=Torphy630&given=Laine739&gender=female', total: 2, corpus: true },
    { q: '/Location?name=A%26A%20HEALTHCARE%20LLC', total: 2, corpus: true },
  ], 'ref-data');
  finalize('4.20', subs);
});

await tryRow('4.21', () => needL('4.21', async () => {
  const { subs, res } = await items('4.21', [
    { q: `/Condition?patient=${LPID}&clinical-status=active`, total: 6 },
    { q: `/Condition?patient=${LPID}&code=http://snomed.info/sct|72892002`, total: 3 },
    { q: `/Observation?patient=${LPID}&code=29463-7&value-quantity=gt60`, total: 2 },
    { q: `/Observation?patient=${LPID}&code-value-quantity=http://loinc.org|8302-2$gt160`, total: 3 },
    { q: `/Immunization?patient=${LPID}&vaccine-code=http://hl7.org/fhir/sid/cvx|140`, total: 3 },
    { q: `/MedicationRequest?patient=${LPID}&status=stopped`, total: 9 },
    { q: `/MedicationRequest?patient=${LPID}&code=http://www.nlm.nih.gov/research/umls/rxnorm|757594`, total: 4 },
  ], 'clinical');
  const w = (res[2].api.json?.entry || []).map((e) => e.resource.valueQuantity?.value).sort();
  subs.push(sub(w.join() === '60.2,64.5', `29463-7 values ${w.join(' and ')} (expected 60.2 and 64.5)`));
  const h = (res[3].api.json?.entry || []).map((e) => e.resource.valueQuantity?.value);
  subs.push(sub(h.length === 3 && h.every((v) => v === 164.1), `8302-2 values ${h.join(',')} (expected all 164.1)`));
  finalize('4.21', subs);
}));

await tryRow('4.22', () => needL('4.22', async () => {
  const { subs, res } = await items('4.22', [
    { q: `/Patient?_id=${LPID}&_revinclude=Immunization:patient`, total: 1, inc: 8 },
    { q: `/Patient?_has:Condition:patient:code=http://snomed.info/sct|706893006&_count=50`, total: { ge: 1 }, has: [LPID] },
    { q: `/Observation?patient=${LPID}&code=29463-7&_sort=date`, total: 4 },
  ], 'revinclude-has-sort');
  const v = (res[2].api.json?.entry || []).map((e) => e.resource.valueQuantity?.value);
  subs.push(sub(v.join() === '55.4,58.5,60.2,64.5', `sorted values top to bottom ${v.join(', ')} (expected 55.4, 58.5, 60.2, 64.5)`));
  finalize('4.22', subs);
}));

// ---------- 8.4 (T3 data) ----------
await tryRow('4.23', async () => {
  const { subs } = await items('4.23', [
    { q: `/Patient?identifier=${SYN}|${PID}`, total: 1, has: [PID] },
    { q: '/Patient?phone=555-613-6236', total: 1, has: [PID] },
    { q: '/Patient?birthdate=2015-12-29', total: 2, corpus: true, has: [PID] },
    { q: '/Patient?address-city=Everett&gender=female', total: 38, corpus: true },
  ], 'patient');
  finalize('4.23', subs);
});
await tryRow('4.24', async () => {
  const { subs } = await items('4.24', [
    { q: `/Encounter?patient=${PID}&class=AMB`, total: 23 },
    { q: `/Encounter?patient=${PID}&class=EMER`, total: 1 },
    { q: `/Encounter?patient=${PID}&date=ge2020`, total: 12 },
  ], 'encounter');
  finalize('4.24', subs);
});
await tryRow('4.25', async () => {
  const { subs, res } = await items('4.25', [
    { q: `/Encounter?patient=${PID}&_include=Encounter:service-provider`, total: 24, noInc: true },
  ], 'unresolved-include');
  const txt = await openFirstRow('4.25b-row-open');
  const sp = (res[0].api.json?.entry || []).map((e) => e.resource.serviceProvider?.reference);
  subs.push(sub(sp.length > 0 && sp.every((x) => /^Organization\?identifier=https:\/\/github.com\/synthetichealth\/synthea\|/.test(x || '')), `serviceProvider.reference left as conditional string (e.g. ${sp[0]}); modal text has it: ${/Organization\?identifier=/.test(txt)}`));
  finalize('4.25', subs);
});
await tryRow('4.26', async () => {
  const { subs } = await items('4.26', [
    { q: `/Condition?patient=${PID}&clinical-status=active`, total: 1 },
    { q: `/Condition?patient=${PID}&code=http://snomed.info/sct|65363002`, total: 2 },
    { q: `/Immunization?patient=${PID}`, total: 25 },
    { q: `/MedicationRequest?patient=${PID}`, total: 7 },
    { q: `/Procedure?patient=${PID}`, total: 18 },
  ], 'clinical');
  finalize('4.26', subs);
});
await tryRow('4.27', async () => {
  const { subs, res } = await items('4.27', [
    { q: `/Observation?patient=${PID}&code=8302-2&_sort=date&_count=20`, total: 15 },
    { q: `/Observation?patient=${PID}&code=8302-2&value-quantity=gt120`, total: 4 },
    { q: `/Observation?patient=${PID}&code=8302-2&value-quantity=gt100||cm`, total: 8 },
  ], 'growth');
  const o = (res[0].api.json?.entry || []).map((e) => e.resource);
  const f = o[0], l = o[o.length - 1];
  const d = (x) => `${x?.valueQuantity?.value} (${(x?.effectiveDateTime || '').slice(0, 10)})`;
  subs.push(sub(o.length === 15 && f?.valueQuantity?.value === 72 && f?.effectiveDateTime?.startsWith('2016-09-07') && l?.valueQuantity?.value === 145.3 && l?.effectiveDateTime?.startsWith('2026-01-06'), `first ${d(f)}, last ${d(l)} (expected 72 cm 2016-09-07, 145.3 cm 2026-01-06)`));
  finalize('4.27', subs);
});
await tryRow('4.28', async () => {
  const { subs } = await items('4.28', [
    { q: `/Organization?identifier=${SYN}|e2a8b444-9b8f-36ff-84c4-05ee98589482`, total: 1, corpus: true },
    { q: '/Organization?name=WHITLEY WELLNESS', total: 1, corpus: true },
    { q: '/Location?address-city=Fitchburg', total: 8, corpus: true },
    { q: '/Location?name=Fitchburg Outpatient Clinic', total: 1, corpus: true },
  ], 'corpus-ref');
  finalize('4.28', subs);
});
await tryRow('4.29', async () => {
  const { subs } = await items('4.29', [
    { q: `/Patient?_has:Condition:patient:code=http://snomed.info/sct|65363002&_id=${PID}`, total: 1 },
    { q: `/Patient?_id=${PID}&_revinclude=Immunization:patient`, total: 1, inc: 25 },
    { q: `/Patient?_id=${PID}&_revinclude=Encounter:patient&_revinclude=Procedure:patient`, total: 1, inc: 42 },
  ], 'has-revinclude');
  finalize('4.29', subs);
});

// ---------- pass criteria ----------
await tryRow('4.pc', async () => {
  const bad = typedRequests.filter((r) => !/_total=/.test(r.url) && !/_summary=count/.test(r.url));
  const sample = typedRequests.slice(-1)[0];
  record(S, '4.pc', bad.length === 0 && typedRequests.length > 0 ? 'pass' : 'fail',
    `${typedRequests.length} UI queries sent; ${bad.length} without _total=accurate${bad[0] ? ' e.g. ' + bad[0].url : ''}; last: ${sample?.url}; page errors: ${consoleErrors.length}${consoleErrors[0] ? ' ' + consoleErrors[0].slice(0, 120) : ''}`);
});

await browser.close();
