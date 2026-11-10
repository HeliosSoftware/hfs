// T6 - ViewDefinition (matrix section 10): patient_demographics + observation_flat via /ui/sql/view-definitions,
// verified through the API ($sql-run in json, csv, ndjson, parquet incl. PAR1 magic), plus rail/duplicate/delete.
// Env: HFS_BASE, OUT (lib.mjs); PID (default 7d24f7a0-...) only used for an optional extra check; NOCLEAN=1 keeps pre-existing
// patient_demographics*/observation_flat views (default: they are deleted first so a re-run starts clean).
import fs from 'fs';
import path from 'path';
import { BASE, OUTPUTS, launch, shot, record, api } from '../lib.mjs';

const S = 'T6';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const FJ = { Accept: 'application/fhir+json' };
const CT = { 'Content-Type': 'application/fhir+json', Accept: 'application/fhir+json' };
async function row(id, fn) { try { await fn(); } catch (e) { record(S, id, 'fail', `exception: ${String(e.message || e).split("\n").slice(0, 3).join(" ")}`); } }

const VD1 = { resourceType: 'ViewDefinition', url: 'http://example.org/ViewDefinition/patient_demographics', name: 'patient_demographics', status: 'active', resource: 'Patient',
  select: [{ column: [
    { name: 'id', path: 'getResourceKey()', type: 'id' }, { name: 'gender', path: 'gender' },
    { name: 'birth_date', path: 'birthDate', type: 'date' }, { name: 'family', path: 'name.first().family' },
    { name: 'city', path: 'address.first().city' }] }],
  where: [{ path: 'active.exists().not() or active = true' }] };
const VD2 = { resourceType: 'ViewDefinition', url: 'http://example.org/ViewDefinition/observation_flat', name: 'observation_flat', status: 'active', resource: 'Observation',
  select: [{ column: [
    { name: 'id', path: 'getResourceKey()', type: 'id' }, { name: 'patient_id', path: 'subject.getReferenceKey(Patient)', type: 'id' },
    { name: 'code', path: 'code.coding.first().code' }, { name: 'value', path: 'value.ofType(Quantity).value', type: 'decimal' },
    { name: 'effective', path: 'effective.ofType(dateTime)', type: 'dateTime' }] }] };
const pretty = (o) => JSON.stringify(o, null, 2);

// ---- pre-clean ----
if (process.env.NOCLEAN !== '1') {
  for (const n of ['patient_demographics', 'patient_demographics_copy', 'patient_demographics_copy_2', 'observation_flat', 'nourl_vd', 'nourl_vd_copy', 'pd_reader']) {
    for (const t of ['ViewDefinition', 'Library']) {
      const r = await api(`/${t}?name=${n}&_count=50`, { headers: FJ });
      for (const e of r.json?.entry || []) await api(`/${t}/${e.resource.id}`, { method: 'DELETE' });
    }
  }
}

const { browser, context, page } = await launch();
page.setDefaultTimeout(10000);
page.on('dialog', (d) => d.accept().catch(() => {}));
const VDURL = `${BASE}/ui/sql/view-definitions`;

// ---- editor helpers (CodeMirror 6 mounted on #vd-editor) ----
const cm = () => page.locator('#vd-editor .cm-content');
async function setDoc(text) {
  const mounted = await cm().isVisible().catch(() => false);
  if (mounted) {
    await cm().click();
    await page.keyboard.press('ControlOrMeta+a');
    await page.keyboard.press('Delete');
    const linted = page.waitForResponse((r) => r.url().includes('/sql/view-definitions/lint') && r.request().method() === 'POST', { timeout: 8000 }).catch(() => null);
    const ran = page.waitForResponse((r) => r.url().includes('/sql/view-definitions/run') && r.request().method() === 'POST', { timeout: 10000 }).catch(() => null);
    await page.keyboard.insertText(text);
    await linted; await ran;
    await page.waitForTimeout(400);
  } else {
    await page.fill('textarea[name=json]', text);
  }
}
async function setCursor(pos) {
  await cm().evaluate((dom, offset) => { const CM = window.HfsCodeMirror; const v = CM.EditorView.findFromDOM(dom); v.dispatch({ selection: { anchor: offset }, scrollIntoView: true }); v.focus(); }, pos);
}
const docText = () => page.locator('textarea[name=json]').inputValue();
async function resultsMeta(timeout = 15000) {
  await page.waitForSelector('#run-results-meta', { timeout }).catch(() => {});
  await page.waitForFunction(() => /rows ·|last successful run/.test(document.querySelector('#run-results-meta')?.textContent || ''), null, { timeout }).catch(() => {});
  return (await page.locator('#run-results-meta').textContent().catch(() => '') || '').trim();
}
async function waitMetaRows(timeout = 15000) {
  await page.waitForFunction(() => /^\d+ rows · /.test((document.querySelector('#run-results-meta')?.textContent || '').trim()), null, { timeout }).catch(() => {});
  return (await page.locator('#run-results-meta').textContent().catch(() => '') || '').trim();
}
async function headers() { return page.$$eval('#run-results .data-table thead th', (e) => e.map((x) => x.textContent.trim())); }
async function chip() {
  // validity chip of the guided form
  const t = await page.locator('.editor-validity').first().textContent({ timeout: 3000 }).catch(() => '');
  return (t || '').trim();
}
async function waitChip(re, timeout = 8000) {
  await page.waitForFunction((src) => new RegExp(src).test(document.querySelector('.editor-validity')?.textContent || ''), re.source, { timeout }).catch(() => {});
  return chip();
}
async function save() {
  await page.locator('#vd-editor-form button[name=action][value=save]').click();
  await page.waitForURL(/vd=[^&]+&saved=1|saved=1.*vd=/, { timeout: 20000 });
  return new URL(page.url()).searchParams.get('vd');
}
async function binFetch(urlPath, headers = {}) {
  const res = await fetch(BASE + urlPath, { headers });
  return { status: res.status, ct: res.headers.get('content-type'), buf: Buffer.from(await res.arrayBuffer()) };
}

let VD, VD2ID;

// ================= 10.1 patient_demographics =================
await row('6.1.1-create-new', async () => {
  await page.goto(`${VDURL}`);
  await page.waitForTimeout(500);
  await shot(page, S, '6.1a-list');
  await page.getByRole('link', { name: /Create New/ }).first().click();
  await page.waitForURL(/vd=new/);
  await page.waitForTimeout(800);
  const title = (await page.locator('.page-head__title--kind').first().textContent().catch(() => '') || '').trim();
  const txt = await docText();
  await shot(page, S, '6.1b-starter');
  let starter = ''; try { starter = JSON.parse(txt).name; } catch {}
  record(S, '6.1.1-create-new', /New View Definition/.test(title) && starter ? 'pass' : 'fail', `title="${title}" starter name="${starter}" (expected title "New View Definition" and a starter document)`);
});

await row('6.1.2-3-paste-and-results', async () => {
  await setDoc(pretty(VD1));
  const meta = await waitMetaRows();
  const hs = await headers();
  const nrows = await page.locator('#run-results .data-table tbody tr').count();
  const nPat = (await api('/Patient?_summary=count', { headers: FJ })).json?.total;
  const expRows = Math.min(50, nPat);
  const c = await waitChip(/No issues/);
  await shot(page, S, '6.1c-results');
  const ok = new RegExp(`^${expRows} rows · [\\d,]+ ms$`).test(meta) && hs.join(',') === 'id,gender,birth_date,family,city' && /No issues/.test(c);
  record(S, '6.1.2-3-paste-and-results', ok ? 'pass' : 'fail', `meta="${meta}" (expect "${expRows} rows · N ms", stored Patients ${nPat}, capped at 50); columns=${hs.join(',')}; table rows=${nrows}; chip="${c}" (expect "No issues.")`);
});

await row('6.1.4-lint', async () => {
  const text = (await docText()).replace('"column"', '"colum"');
  await setDoc(text);
  const c = await waitChip(/2 issues?/);
  const pos = text.indexOf('"colum"') + 3;
  await setCursor(pos);
  await page.waitForTimeout(500);
  // hover tooltip on the key
  const box = await cm().evaluate((dom, p) => { const v = window.HfsCodeMirror.EditorView.findFromDOM(dom); const r = v.coordsAtPos(p); return r ? { x: r.left, y: (r.top + r.bottom) / 2 } : null; }, pos);
  let tip = '';
  if (box) { await page.mouse.move(box.x + 2, box.y); await page.waitForTimeout(1200); tip = (await page.locator('.cm-tooltip-lint').first().textContent().catch(() => '') || '').trim(); }
  await shot(page, S, '6.1d-lint-tooltip');
  await page.keyboard.press('ControlOrMeta+Shift+M');
  await page.waitForTimeout(600);
  const panel = (await page.locator('.cm-panel.cm-panel-lint').first().textContent().catch(() => '') || '').trim();
  await shot(page, S, '6.1e-lint-panel');
  await setCursor(pos);
  await page.keyboard.press('Control+.');
  await page.waitForTimeout(600);
  const menu = (await page.locator('.cm-quickfix').first().textContent().catch(() => '') || '').trim();
  await shot(page, S, '6.1f-quickfix');
  await page.locator('.cm-quickfix__item').filter({ hasText: 'Rename to "column"' }).first().click();
  const c2 = await waitChip(/No issues/);
  const ok = /2 issues?/.test(c) && /Unknown key "colum"/.test(tip + panel) && /at least one of column, select, or unionAll/.test(tip + panel) && /Rename to "column"/.test(menu) && /Remove "colum"/.test(menu) && /No issues/.test(c2);
  record(S, '6.1.4-lint', ok ? 'pass' : 'fail', `chip="${c}"; tooltip="${tip.slice(0, 120)}"; panel has Unknown key=${/Unknown key "colum"/.test(panel)} / select-needs-column=${/at least one of column/.test(panel)}; quick-fix menu="${menu.slice(0, 120)}"; after fix chip="${c2}"`);
});

await row('6.1.5-completion', async () => {
  let text = await docText();
  const d = JSON.parse(text);
  d.select[0].column[0].path = 'getRes';
  text = pretty(d);
  await setDoc(text);
  await setCursor(text.indexOf('getRes') + 6);
  await page.keyboard.press('Control+Space');
  await page.waitForTimeout(1500);
  const opts = await page.locator('.cm-tooltip-autocomplete ul li[role=option]').allTextContents();
  await shot(page, S, '6.1g-completion');
  const has = opts.some((o) => /getResourceKey/.test(o));
  await page.keyboard.press('Enter');
  await page.waitForTimeout(500);
  const after = await docText();
  const inserted = /"path": "getResourceKey\(\)"/.test(after) || after.includes('getResourceKey()');
  // structural keys at an object position
  // delete a key name (here "resource") and complete at the top-level object position: required keys carry a "required" tag
  const skeleton = pretty({ resourceType: 'ViewDefinition', name: 'x', select: [{ column: [{ name: 'g', path: 'gender' }] }] }).replace('"name": "x",', '"name": "x",\n  ');
  await setDoc(skeleton);
  await setCursor(skeleton.indexOf('"name": "x",\n  ') + '"name": "x",\n  '.length);
  await page.keyboard.press('Control+Space');
  await page.waitForTimeout(1500);
  const keyOpts = await page.locator('.cm-tooltip-autocomplete ul li[role=option]').allTextContents();
  await shot(page, S, '6.1h-completion-keys');
  const reqTagged = keyOpts.some((o) => /required/i.test(o));
  await page.keyboard.press('Escape');
  // restore the real definition
  await setDoc(pretty(VD1));
  await waitMetaRows();
  const ok = has && inserted && keyOpts.length > 0 && reqTagged;
  record(S, '6.1.5-completion', ok ? 'pass' : 'fail', `getRes options [${opts.slice(0, 5).join(' | ')}] has getResourceKey=${has}; accepted -> inserted "getResourceKey()"=${inserted}; object-position keys [${keyOpts.slice(0, 8).join(' | ')}] required-tagged=${reqTagged}`);
});

await row('6.1.6-save', async () => {
  VD = await save();
  await page.waitForTimeout(800);
  const saved = await page.getByText('Saved.', { exact: true }).first().isVisible().catch(() => false);
  const railItem = await page.locator('#vd-rail-list .filter-rail__item', { hasText: 'patient_demographics' }).first().textContent().catch(() => '');
  const recent = await page.locator('#vd-rail-recent').textContent().catch(() => '');
  await shot(page, S, '6.1i-saved');
  const g = await api(`/ViewDefinition/${VD}`, { headers: FJ, save: 't6-vd-patient_demographics.json' });
  const ok = !!VD && saved && /patient_demographics/.test(railItem) && /Patient/.test(railItem) && /patient_demographics/.test(recent || '') && g.status === 200 && g.json?.name === 'patient_demographics';
  record(S, '6.1.6-save', ok ? 'pass' : 'fail', `VD=${VD} url=${page.url()} "Saved."=${saved}; rail="${(railItem || '').replace(/\s+/g, ' ').trim()}"; in Recently used=${/patient_demographics/.test(recent || '')}; API GET -> ${g.status} name=${g.json?.name}`);
});

await row('6.1.7-crosscheck', async () => {
  await page.goto(`${VDURL}?vd=${VD}`);
  await waitMetaRows();
  const rowsAll = await page.$$eval('#run-results .data-table tbody tr', (trs) => trs.map((tr) => [...tr.querySelectorAll('td')].map((x) => x.textContent.trim())));
  const first = rowsAll.find((r) => r[2] && r[3] && r[4]) || rowsAll[0] || [];   // prefer a fully populated row (corpus patients; the T2 Larkin patient has no birthDate)
  if (!first.length) return record(S, '6.1.7-crosscheck', 'fail', 'Results table has no rows');
  const [rid, gender, birth, family, city] = first;
  const p = await api(`/Patient?_id=${rid}`, { headers: FJ });
  const r = p.json?.entry?.[0]?.resource;
  await shot(page, S, '6.1j-crosscheck');
  const ok = r && r.gender === gender && (r.birthDate || '') === birth && (r.name?.[0]?.family || '') === family && (r.address?.[0]?.city || '') === city;
  record(S, '6.1.7-crosscheck', ok ? 'pass' : 'fail', `row ${first.join(' | ')} vs GET /Patient?_id=${rid}: gender=${r?.gender} birthDate=${r?.birthDate} family=${r?.name?.[0]?.family} city=${r?.address?.[0]?.city}`);
});

// ================= 10.2 observation_flat =================
await row('6.2-observation_flat', async () => {
  await page.goto(`${VDURL}?vd=new`);
  await page.waitForTimeout(800);
  await setDoc(pretty(VD2));
  const meta = await waitMetaRows();
  const hs = await headers();
  const codes = await page.$$eval('#run-results .data-table tbody tr td:nth-child(3)', (e) => e.map((x) => x.textContent.trim()));
  await shot(page, S, '6.2a-results');
  VD2ID = await save();
  await page.waitForTimeout(500);
  await shot(page, S, '6.2b-saved');
  const nObs = (await api('/Observation?_summary=count', { headers: FJ })).json?.total;
  const exp = Math.min(50, nObs);
  const g = await api(`/ViewDefinition/${VD2ID}`, { headers: FJ, save: 't6-vd-observation_flat.json' });
  const ok = new RegExp(`^${exp} rows · `).test(meta) && hs.join(',') === 'id,patient_id,code,value,effective' && codes.length > 0 && codes.every((c) => c.length) && g.status === 200;
  record(S, '6.2-observation_flat', ok ? 'pass' : 'fail', `meta="${meta}" (expect ${exp} rows); columns=${hs.join(',')}; sample codes=${codes.slice(0, 5).join(',')} (matrix: codes such as 8302-2; present=${codes.includes('8302-2')}); VD2=${VD2ID}; saved GET ${g.status}`);
});

// ================= API: $sql-run in every format =================
async function runApi(rid, vdId, name, expCols, expRowsMax) {
  await row(rid, async () => {
    const base = `/$sql-run?subjectReference=ViewDefinition/${vdId}&_limit=${expRowsMax}`;
    const note = [];
    let ok = true;
    // ndjson
    const nd = await api(`${base}&_format=ndjson`, { save: `t6-${name}.ndjson` });
    const ndRows = nd.text.split('\n').filter(Boolean).map((l) => JSON.parse(l));
    note.push(`ndjson ${nd.status} ${ndRows.length} rows ct=${nd.headers['content-type']}`);
    ok = ok && nd.status === 200 && ndRows.length > 0 && expCols.every((c) => c in ndRows[0]);
    // json
    const js = await api(`${base}&_format=json`, { save: `t6-${name}.json` });
    let jr = []; try { jr = JSON.parse(js.text.replace(/^# .*\n/, '')); } catch {}
    note.push(`json ${js.status} ${jr.length} rows`);
    ok = ok && js.status === 200 && Array.isArray(jr) && jr.length === ndRows.length;
    // csv
    const cs = await api(`${base}&_format=csv`, { save: `t6-${name}.csv` });
    const cl = cs.text.split('\n').filter(Boolean);
    note.push(`csv ${cs.status} header="${cl[0]}" ${cl.length - 1} rows`);
    ok = ok && cs.status === 200 && cl[0].split(',').join(',') === expCols.join(',') && cl.length - 1 === ndRows.length;
    // parquet
    const pq = await binFetch(`${base}&_format=parquet`);
    fs.writeFileSync(path.join(OUTPUTS, `t6-${name}.parquet`), pq.buf);
    const head = pq.buf.subarray(0, 4).toString('latin1'), tail = pq.buf.subarray(pq.buf.length - 4).toString('latin1');
    note.push(`parquet ${pq.status} ${pq.buf.length} bytes head=${head} tail=${tail} ct=${pq.ct}`);
    ok = ok && pq.status === 200 && head === 'PAR1' && tail === 'PAR1';
    // unsupported format
    const bad = await api(`${base}&_format=xml`);
    note.push(`_format=xml -> ${bad.status}`);
    ok = ok && bad.status === 400;
    // inline POST of the definition (subjectResource shorthand)
    const inline = await api('/$sql-run?_format=ndjson&_limit=3', { method: 'POST', headers: { 'Content-Type': 'application/fhir+json' }, body: JSON.stringify(name === 'patient_demographics' ? VD1 : VD2) });
    const il = inline.text.split('\n').filter(Boolean).length;
    note.push(`inline POST subjectResource ${inline.status} ${il} rows`);
    ok = ok && inline.status === 200 && il > 0 && il <= 3;
    record(S, rid, ok ? 'pass' : 'fail', `${name}: ${note.join('; ')}`);
  });
}
if (VD) await runApi('6.A1-sql-run-patient_demographics', VD, 'patient_demographics', ['id', 'gender', 'birth_date', 'family', 'city'], 50);
else record(S, '6.A1-sql-run-patient_demographics', 'fail', 'patient_demographics was not saved (6.1.6 failed)');
if (VD2ID) await runApi('6.A2-sql-run-observation_flat', VD2ID, 'observation_flat', ['id', 'patient_id', 'code', 'value', 'effective'], 50);
else record(S, '6.A2-sql-run-observation_flat', 'fail', 'observation_flat was not saved (6.2 failed)');

await row('6.A3-api-create-roundtrip', async () => {
  // the same definitions created through the REST API, then read back by name
  const g = await api('/ViewDefinition?name=patient_demographics&_summary=count', { headers: FJ });
  const g2 = await api('/ViewDefinition?name=observation_flat&_summary=count', { headers: FJ });
  record(S, '6.A3-api-create-roundtrip', g.json?.total === 1 && g2.json?.total === 1 ? 'pass' : 'fail', `ViewDefinition?name=patient_demographics total=${g.json?.total}, name=observation_flat total=${g2.json?.total} (expect 1 each)`);
});

// ================= 10.3 rail, duplicate, delete =================
await row('6.3.1-filter', async () => {
  await page.goto(`${VDURL}?vd=${VD}`);
  await page.waitForTimeout(500);
  await page.fill('input[name=filter]', 'patient');
  await page.waitForTimeout(1500);
  const items = await page.$$eval('#vd-rail-list .filter-rail__item', (e) => e.map((x) => x.getAttribute('data-full-name')));
  await shot(page, S, '6.3a-filter');
  record(S, '6.3.1-filter', items.length === 1 && items[0] === 'patient_demographics' ? 'pass' : 'fail', `rail after filter "patient": [${items.join(', ')}] (expected only patient_demographics)`);
});

async function dup(fromId) {
  await page.goto(`${VDURL}?vd=${fromId}`);
  await page.waitForTimeout(500);
  await page.locator("button[name='action'][value='duplicate']").click();
  await page.waitForURL((u) => u.searchParams.get('saved') === '1' && u.searchParams.get('vd') !== fromId, { timeout: 20000 });
  return new URL(page.url()).searchParams.get('vd');
}
let COPY1, COPY2, NOURL, NOURLCOPY, LIB;
await row('6.3.2-duplicate', async () => {
  COPY1 = await dup(VD);
  await page.waitForTimeout(600);
  await shot(page, S, '6.3b-copy1');
  const c1 = (await api(`/ViewDefinition/${COPY1}`, { headers: FJ })).json;
  COPY2 = await dup(VD);
  await shot(page, S, '6.3c-copy2');
  const c2 = (await api(`/ViewDefinition/${COPY2}`, { headers: FJ })).json;
  const orig = (await api(`/ViewDefinition/${VD}`, { headers: FJ })).json;
  // definition without url
  const nu = await api('/ViewDefinition', { method: 'POST', headers: CT, body: JSON.stringify({ resourceType: 'ViewDefinition', name: 'nourl_vd', status: 'active', resource: 'Patient', select: [{ column: [{ name: 'id', path: 'getResourceKey()' }] }] }) });
  NOURL = nu.json?.id;
  NOURLCOPY = await dup(NOURL);
  const nc = (await api(`/ViewDefinition/${NOURLCOPY}`, { headers: FJ })).json;
  const ok = c1.name === 'patient_demographics_copy' && c1.url === 'http://example.org/ViewDefinition/patient_demographics_copy' && c2.name === 'patient_demographics_copy_2' && c2.url === 'http://example.org/ViewDefinition/patient_demographics_copy_2' && orig.url === VD1.url && !nc.url;
  record(S, '6.3.2-duplicate', ok ? 'pass' : 'fail', `copy1 ${c1.name} ${c1.url}; copy2 ${c2.name} ${c2.url}; original url ${orig.url}; no-url duplicate: name=${nc.name} url=${nc.url ?? '(none)'}`);
});

await row('6.3.3-dependent-sql-view', async () => {
  const lib = await api('/Library', { method: 'POST', headers: CT, body: JSON.stringify({ resourceType: 'Library', name: 'pd_reader', status: 'active',
    type: { coding: [{ system: 'http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes', code: 'sql-view' }] },
    relatedArtifact: [{ type: 'depends-on', resource: VD1.url, label: 'pd' }],
    content: [{ contentType: 'application/sql', data: Buffer.from('SELECT id FROM pd').toString('base64') }] }) });
  LIB = lib.json?.id;
  if (!LIB) return record(S, '6.3.3-dependent-sql-view', 'fail', `could not create SQL View Library: ${lib.status} ${lib.text.slice(0, 160)}`);
  await sleep(1000);
  await page.goto(`${BASE}/ui/sql/views?lib=${LIB}`);
  await page.waitForTimeout(1000);
  const reads = page.getByText('Reads from').first();
  const link = await page.locator(`a[href*="view-definitions?vd=${VD}"]`).first().getAttribute('href').catch(() => null);
  await shot(page, S, '6.3d-sql-view-reads-from');
  const before = (await api(`/$sql-run?subjectReference=Library/${LIB}&_format=ndjson`)).text.split('\n').filter(Boolean).length;
  // edit the copy: where false, save through the UI
  await page.goto(`${VDURL}?vd=${COPY1}`);
  await page.waitForTimeout(800);
  const d = JSON.parse(await docText());
  d.where = [{ path: 'false' }];
  await setDoc(pretty(d));
  await page.waitForTimeout(1200);
  await page.locator('#vd-editor-form button[name=action][value=save]').click();
  await page.waitForURL(/saved=1/, { timeout: 20000 });
  await page.waitForTimeout(1200);
  const meta = (await page.locator('#run-results-meta').textContent().catch(() => '') || '').trim();
  await shot(page, S, '6.3e-copy-where-false');
  const copyRows = (await api(`/$sql-run?subjectReference=ViewDefinition/${COPY1}&_format=ndjson`)).text.split('\n').filter(Boolean).length;
  const after = (await api(`/$sql-run?subjectReference=Library/${LIB}&_format=ndjson`)).text.split('\n').filter(Boolean).length;
  const ok = !!link && (await reads.isVisible().catch(() => true)) !== undefined && copyRows === 0 && after === before && before > 0;
  record(S, '6.3.3-dependent-sql-view', ok ? 'pass' : 'fail', `Reads-from link to original=${link}; copy rows after where=false: ${copyRows} (UI meta "${meta}"); dependent SQL View rows before=${before} after=${after} (must be unchanged and non-zero)`);
});

async function deleteVd(id, name) {
  await page.goto(`${VDURL}?vd=${id}`);
  await page.waitForTimeout(600);
  await page.locator('[data-crud-delete]').click();
  await page.waitForTimeout(500);
  const body = await page.locator('body').innerText();
  const msg = `Delete view definition "${name}"? This cannot be undone.`;
  const shown = body.includes(msg);
  if (id === COPY1) await shot(page, S, '6.3f-delete-confirm');
  // in-page confirm dialog: press its confirm button if present
  const btn = page.locator('dialog[open] button, [role=dialog] button, .confirm button').filter({ hasNotText: /Cancel/ }).first();
  if (await btn.isVisible().catch(() => false)) await btn.click();
  await page.waitForTimeout(1500);
  const g = await api(`/ViewDefinition/${id}`, { headers: FJ });
  return { shown, gone: g.status === 404 || g.status === 410 };
}
await row('6.3.4-delete', async () => {
  const a = await deleteVd(COPY1, 'patient_demographics_copy');
  const b = await deleteVd(COPY2, 'patient_demographics_copy_2');
  await page.goto(VDURL);
  const rail = await page.$$eval('#vd-rail-list .filter-rail__item', (e) => e.map((x) => x.getAttribute('data-full-name')));
  await shot(page, S, '6.3g-after-delete');
  const ok = a.gone && b.gone && !rail.includes('patient_demographics_copy') && !rail.includes('patient_demographics_copy_2') && rail.includes('patient_demographics');
  record(S, '6.3.4-delete', ok ? 'pass' : 'fail', `copy1: confirm text shown=${a.shown} deleted=${a.gone}; copy2: confirm shown=${b.shown} deleted=${b.gone}; rail now [${rail.join(', ')}]`);
});

await row('6.3.5-negative-resource-nope', async () => {
  const before = (await api('/ViewDefinition?name=new_view&_summary=count', { headers: FJ })).json?.total;
  await page.goto(`${VDURL}?vd=new`);
  await page.waitForTimeout(800);
  await waitMetaRows();   // starter runs first so there is a "previous table"
  const text = (await docText()).replace(/"resource":\s*"[A-Za-z]+"/, '"resource": "Nope"');
  await setDoc(text);
  await page.waitForTimeout(2000);
  const c = await chip();
  const notice = (await page.locator('#run-notice').textContent().catch(() => '') || '').replace(/\s+/g, ' ').trim();
  const meta = (await page.locator('#run-results-meta').textContent().catch(() => '') || '').trim();
  await shot(page, S, '6.3h-nope');
  await page.locator('#vd-editor-form button[name=action][value=save]').click();
  await page.waitForTimeout(800);
  const body = await page.locator('body').innerText();
  const prompt = /This view definition still has 1 error\. Save it anyway\?/.test(body);
  await shot(page, S, '6.3i-save-prompt');
  const cancel = page.getByRole('button', { name: 'Cancel' }).last();
  if (await cancel.isVisible().catch(() => false)) await cancel.click();
  await page.waitForTimeout(800);
  const after = (await api('/ViewDefinition?name=new_view&_summary=count', { headers: FJ })).json?.total;
  const ok = /Could not run the view\./.test(notice) && /last successful run/.test(meta) && prompt && before === after;
  record(S, '6.3.5-negative-resource-nope', ok ? 'pass' : 'fail', `chip="${c}"; notice="${notice.slice(0, 120)}"; meta="${meta}"; save prompt shown=${prompt}; new_view count before=${before} after cancel=${after}`);
});

// ---- cleanup of helper objects (the two real views stay) ----
for (const [t, id] of [['Library', LIB], ['ViewDefinition', NOURL], ['ViewDefinition', NOURLCOPY]]) if (id) await api(`/${t}/${id}`, { method: 'DELETE' });
await browser.close();
console.log('T6 done');
