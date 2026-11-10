// T5 - Bulk Data $export from the Export page (matrix section 9, rows 5.1-5.13) plus API cross-checks (5.A*).
// Env: HFS_BASE, OUT (lib.mjs)
//   PID            patient id used by 5.2/5.3 and the Group fixture   (default 7d24f7a0-6f2e-ce3b-5568-db7b14695583)
//   PID_QUERY      text typed in the Patients picker for 5.2          (default Cari853)
//   T3_START       instant (UTC, ...Z) the T3 submission was created; falls back to an instant between the two oldest Patients
//   T3_END         instant T3 finished; falls back to 1s before RiskAssessment manual-risk's lastUpdated
//   COUNTS         corpus-counts.tsv (default /home/edson/hfs-t3/corpus/corpus-counts.tsv)
//   FULL=1|0       force matrix-literal expectations on/off (default: auto, on when Patient total == corpus Patient + 1)
//   EXPORT_TIMEOUT_MS  per-export wait (default 900000)
//   SKIP_CANCEL_ALL=1 skip the API "everything" cancel (5.A5)
import fs from 'fs';
import path from 'path';
import { BASE, OUTPUTS, launch, shot, record, api } from '../lib.mjs';

const S = 'T5';
const PID = process.env.PID || '7d24f7a0-6f2e-ce3b-5568-db7b14695583';
const PID_QUERY = process.env.PID_QUERY || 'Cari853';
const COUNTS = process.env.COUNTS || '/home/edson/hfs-t3/corpus/corpus-counts.tsv';
const TMO = Number(process.env.EXPORT_TIMEOUT_MS || 900000);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const lines = (t) => t.split('\n').filter((l) => l.trim());
const decode = (s) => s.replace(/&amp;/g, '&').replace(/&#x2F;/g, '/').replace(/&#38;/g, '&').replace(/&quot;/g, '"');

const counts = {};
try { for (const l of fs.readFileSync(COUNTS, 'utf8').split('\n')) { const [k, v] = l.split('\t'); if (k && v) counts[k] = Number(v); } } catch {}

async function total(type, q = '') {
  const r = await api(`/${type}?${q}${q ? '&' : ''}_summary=count`, { headers: { Accept: 'application/fhir+json' } });
  return r.json?.total ?? NaN;
}
async function row(id, fn) {
  try { await fn(); } catch (e) { record(S, id, 'fail', `exception: ${String(e.message || e).split('\n')[0]}`); }
}
const chunks = (n) => Math.ceil(n / 1000);

// ---------- UI helpers ----------
let page;
async function cardIds() {
  return page.$$eval('.job-card', (els) => els.map((e) => e.id.replace(/^job-/, '')));
}
async function pickFixed(rootId, nativeName, value, label) {
  const trigger = page.locator(`#${rootId} [data-combobox-trigger]`);
  if (await trigger.isVisible().catch(() => false)) {
    await trigger.click();
    await page.locator(`#${rootId} [role=option]`).filter({ hasText: label }).first().click();
  } else {
    await page.evaluate(([n, v]) => { const s = document.querySelector(`select[name=${n}]`); s.value = v; s.dispatchEvent(new Event('change', { bubbles: true })); }, [nativeName, value]);
  }
}
const SINCE = { '': 'All time', day: 'Last day', week: 'Last 7 days', month: 'Last 4 weeks', custom: 'Custom' };
async function fillForm({ name, scope, groupId, patientText, patientPick, patientRaw, types, elements, typeFilter, since, sinceCustom, until }) {
  await page.goto(`${BASE}/ui/bulk-export/new`);
  await page.waitForSelector('form.bulk-export-form');
  await page.waitForTimeout(400);
  if (name !== undefined) await page.fill('input[name=name]', name);
  if (scope) await page.locator(`input[name=scope][value=${scope}]`).check();
  if (groupId) await page.fill('input[name=group_id]', groupId);
  if (patientText) {
    const input = page.locator('#bulk-export-patients input[role=combobox]');
    if (await input.isVisible().catch(() => false)) {
      await input.fill(patientText);
      const opt = page.locator('#bulk-export-patients [role=option]').filter({ hasText: patientPick || patientText }).first();
      await opt.waitFor({ timeout: 15000 });
      await opt.click();
    } else {
      await page.fill('textarea[name=patient]', patientRaw || `Patient/${PID}`);
    }
  }
  if (types) {
    const all = page.locator('input[name=all_types]');
    if (await all.isChecked()) await all.uncheck();
    for (const t of types) await page.locator(`input[name=types][value="${t}"]`).check();
  }
  if (elements) await page.fill('input[name=elements]', elements);
  if (typeFilter) await page.fill('input[name=type_filter]', typeFilter);
  if (since !== undefined) await pickFixed('bulk-export-since', 'since_preset', since, SINCE[since]);
  if (sinceCustom) await page.fill('input[name=since_custom]', sinceCustom);
  if (until) await page.fill('input[name=until]', until);
}
async function submit() {
  await page.locator('form.bulk-export-form button[type=submit]').click();
}
async function startExport(opts) {
  const before = new Set(await (async () => { await page.goto(`${BASE}/ui/bulk-export`); return cardIds(); })());
  await fillForm(opts);
  await page.evaluate(() => window.scrollTo(0, 0));
  await shot(page, S, `${opts.rowId}a-form`);
  await submit();
  await page.waitForURL(/\/ui\/bulk-export(\?.*)?$/, { timeout: 30000 });
  await page.waitForSelector('.job-card');
  const now = await cardIds();
  const fresh = now.filter((i) => !before.has(i));
  if (fresh.length !== 1) throw new Error(`expected 1 new card, got ${fresh.length}`);
  return fresh[0];
}
async function cardState(id) {
  const r = await api(`/ui/bulk-export/active/${id}/card`);
  const status = (r.text.match(/class="tag tag--([a-z-]+)"/) || [])[1] || '?';
  const text = r.text.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim();
  return { status, text, html: r.text };
}
async function waitDone(id) {
  const t0 = Date.now();
  for (;;) {
    const s = await cardState(id);
    if (s.status !== 'in-progress') return s;
    if (Date.now() - t0 > TMO) return s;
    await sleep(2000);
  }
}
async function files(id) {
  const r = await api(`/ui/bulk-export/active/${id}`);
  const out = [];
  const re = /<a href="([^"]+)"\s+download="([^"]+)"/g;
  let m;
  while ((m = re.exec(r.text))) out.push({ href: decode(m[1]), label: decode(m[2]), type: decode(m[2]).split('-')[0] });
  return { files: out, html: r.text };
}
async function fetchAll(fl) {
  const res = {};
  for (const f of fl) {
    const r = await api(f.href);
    res[f.label] = { status: r.status, lines: lines(r.text), text: r.text };
  }
  return res;
}
async function showCard(id, shotName) {
  await page.goto(`${BASE}/ui/bulk-export`);
  const c = page.locator(`#job-${id}`);
  await c.scrollIntoViewIfNeeded().catch(() => {});
  await shot(page, S, shotName);
}
async function showDetail(id, shotName) {
  await page.goto(`${BASE}/ui/bulk-export/active/${id}`);
  await shot(page, S, shotName, { full: true });
}

// ---------- start ----------
const { browser, page: pg } = await launch();
page = pg;
const ids = {};

// environment facts
const nPatient = await total('Patient');
const full = process.env.FULL ? process.env.FULL === '1' : (counts.Patient != null && nPatient === counts.Patient + 1);
console.log(`server Patient total=${nPatient}, corpus Patient=${counts.Patient}, full-matrix mode=${full}`);

// ---------- 9.1 Group fixture ----------
await row('5.0-group-fixture', async () => {
  await page.goto(`${BASE}/ui/editor?type=Group`);
  await page.waitForSelector('#editor-source', { state: 'attached' });
  await page.waitForTimeout(800);
  const doc = JSON.stringify({ resourceType: 'Group', id: 'manual-group', type: 'person', actual: true, member: [{ entity: { reference: `Patient/${PID}` } }] });
  const cm = page.locator('.code-editor--resource .cm-content');
  if (await cm.isVisible().catch(() => false)) {
    await cm.click(); await page.keyboard.press('ControlOrMeta+a'); await page.keyboard.press('Delete'); await page.keyboard.insertText(doc);
  } else await page.fill('#editor-source', doc);
  await page.waitForTimeout(800);
  await shot(page, S, '5.0a-group-editor');
  await page.getByRole('button', { name: 'Save Changes' }).click();
  await page.waitForTimeout(800);
  const nv = page.getByRole('button', { name: 'Save new version' });
  const rerun = await nv.isVisible().catch(() => false);   // only on a re-run: Group/manual-group already exists
  if (rerun) { await shot(page, S, '5.0a2-already-exists'); await nv.click(); }
  for (let k = 0; k < 8 && await page.getByText('Unsaved changes').isVisible().catch(() => false); k++) await page.waitForTimeout(1000);
  const pill = await page.getByText('Unsaved changes').isVisible().catch(() => false);
  await shot(page, S, '5.0b-group-saved');
  const g = await api('/Group/manual-group', { headers: { Accept: 'application/fhir+json' } });
  const ok = g.status === 200 && g.json?.member?.[0]?.entity?.reference === `Patient/${PID}`;
  record(S, '5.0-group-fixture', ok && !pill ? 'pass' : 'fail', `GET Group/manual-group -> ${g.status}, member=${g.json?.member?.[0]?.entity?.reference}, Unsaved pill visible=${pill}${rerun ? ' (re-run: confirmed Save new version)' : ''}`);
});

// ---------- 5.6 negative (empty name) ----------
await row('5.6', async () => {
  await page.goto(`${BASE}/ui/bulk-export/new`);
  await page.waitForSelector('form.bulk-export-form');
  await page.waitForTimeout(400);
  await submit();
  await page.waitForTimeout(600);
  const msg = page.locator('#bulk-export-name-error');
  const vis = await msg.isVisible().catch(() => false);
  const txt = (await msg.textContent().catch(() => '') || '').trim();
  await shot(page, S, '5.6a-name-required');
  const url = page.url();
  const ok = vis && txt === 'Enter a name for this export.' && /\/ui\/bulk-export\/new/.test(url);
  record(S, '5.6', ok ? 'pass' : 'fail', `message visible=${vis} text="${txt}" url=${url} (expected "Enter a name for this export." on /new)`);
  // no-JS variant: server renders 400 with the same text
  const ctx = await browser.newContext({ javaScriptEnabled: false });
  const p2 = await ctx.newPage();
  const resp = await p2.goto(`${BASE}/ui/bulk-export/new`);
  await p2.locator('input[name=name]').fill('');
  const [r2] = await Promise.all([p2.waitForResponse((r) => r.request().method() === 'POST'), p2.locator('form.bulk-export-form button[type=submit]').click()]);
  await p2.waitForLoadState();
  const body = await p2.content();
  record(S, '5.6-nojs', r2.status() === 400 && body.includes('Enter a name for this export.') ? 'pass' : 'fail', `no-JS POST -> ${r2.status()}, message present=${body.includes('Enter a name for this export.')} (expected 400 + message)`);
  await ctx.close();
});

// ---------- Clear button on the type grid ----------
await row('5.0-clear', async () => {
  await page.goto(`${BASE}/ui/bulk-export/new`);
  await page.waitForSelector('form.bulk-export-form');
  await page.waitForTimeout(500);
  await page.fill('input[name=name]', 'clear-test');
  await page.fill('input[name=elements]', 'id,gender');
  await page.locator('input[name=types][value=Patient]').check();
  await page.locator('[data-clear-types]').click();
  const st = await page.evaluate(() => ({
    all: document.querySelector('input[name=all_types]').checked,
    any: [...document.querySelectorAll('input[name=types]')].some((i) => i.checked),
    disabled: [...document.querySelectorAll('input[name=types]')].some((i) => i.disabled),
    name: document.querySelector('input[name=name]').value,
    elements: document.querySelector('input[name=elements]').value,
  }));
  await shot(page, S, '5.0c-clear');
  await page.locator('input[name=types][value=Patient]').check();
  const usable = await page.locator('input[name=types][value=Patient]').isChecked();
  const ok = !st.all && !st.any && !st.disabled && st.name === 'clear-test' && st.elements === 'id,gender' && usable;
  record(S, '5.0-clear', ok ? 'pass' : 'fail', `after Clear: allResources=${st.all} anyType=${st.any} anyDisabled=${st.disabled} name="${st.name}" elements="${st.elements}" typeUsable=${usable}`);
});


// ---------- Clear after a validation rejection, and no-JS behaviour ----------
await row('5.0-clear-after-reject', async () => {
  await page.goto(`${BASE}/ui/bulk-export/new`);
  await page.waitForSelector('form.bulk-export-form');
  await page.waitForTimeout(500);
  await page.fill('input[name=name]', 'reject-then-clear');
  await page.locator('input[name=scope][value=system]').check();
  await pickFixed('bulk-export-since', 'since_preset', 'custom', 'Custom');
  await page.fill('input[name=since_custom]', '2026-02-30T00:00:00Z');
  await submit();
  await page.waitForTimeout(1200);
  const errSel = page.locator('#bulk-export-since-custom-error');
  const errBefore = await errSel.isVisible().catch(() => false);
  await page.locator('input[name=types][value=Patient]').check();
  await page.locator('[data-clear-types]').click();
  const st = await page.evaluate(() => ({ all: document.querySelector('input[name=all_types]').checked, any: [...document.querySelectorAll('input[name=types]')].some((i) => i.checked), name: document.querySelector('input[name=name]').value, custom: document.querySelector('input[name=since_custom]').value }));
  const errAfter = await errSel.isVisible().catch(() => false);
  await page.evaluate(() => window.scrollTo(0, 0));
  await shot(page, S, '5.0d-clear-after-reject');
  await page.locator('input[name=types][value=Organization]').check();
  const usable = await page.locator('input[name=types][value=Organization]').isChecked();
  record(S, '5.0-clear-after-reject', errBefore && errAfter && !st.all && !st.any && st.name === 'reject-then-clear' && st.custom === '2026-02-30T00:00:00Z' && usable ? 'pass' : 'fail', `rejection visible before Clear=${errBefore}, after Clear=${errAfter}; allResources=${st.all} anyType=${st.any} name="${st.name}" customInstant="${st.custom}" typeUsable=${usable}`);
});
await row('5.0-nojs-clear-hidden', async () => {
  const ctx = await browser.newContext({ javaScriptEnabled: false });
  const p2 = await ctx.newPage();
  await p2.goto(`${BASE}/ui/bulk-export/new`);
  const hidden = !(await p2.locator('[data-clear-types]').isVisible());
  await p2.fill('input[name=name]', 'nojs-reject');
  await p2.locator('input[name=scope][value=system]').check();
  await p2.evaluate(() => { document.querySelector('select[name=since_preset]').value = 'custom'; const i = document.querySelector('input[name=since_custom]'); i.disabled = false; i.value = '2026-02-30T00:00:00Z'; });
  const [r2] = await Promise.all([p2.waitForResponse((r) => r.request().method() === 'POST'), p2.locator('form.bulk-export-form button[type=submit]').click()]);
  await p2.waitForLoadState();
  const body = await p2.content();
  const stillHidden = !(await p2.locator('[data-clear-types]').isVisible());
  const keptName = (await p2.locator('input[name=name]').inputValue()) === 'nojs-reject';
  await p2.evaluate(() => { document.querySelector('input[name=all_types]').checked = false; });
  const allOff = !(await p2.locator('input[name=all_types]').isChecked());
  await shot(p2, S, '5.0e-nojs-rejected');
  record(S, '5.0-nojs-clear-hidden', hidden && stillHidden && r2.status() === 400 && body.includes('Enter a valid FHIR instant') && keptName && allOff ? 'pass' : 'fail', `Clear hidden without JS=${hidden}, still hidden on the rejected page=${stillHidden}; POST ${r2.status()} message present=${body.includes('Enter a valid FHIR instant')}; name kept=${keptName}; All Resources unchecked manually=${allOff}`);
  await ctx.close();
});

// ---------- Patients picker (matrix 9.2: N matches, scroll for more, eight rows at a time) ----------
async function pickerProbe(term) {
  await page.goto(`${BASE}/ui/bulk-export/new`);
  await page.waitForSelector('form.bulk-export-form');
  await page.waitForTimeout(500);
  await page.locator('input[name=scope][value=patient]').check();
  const input = page.locator('#bulk-export-patients input[role=combobox]');
  await input.click();
  await input.pressSequentially(term, { delay: 40 });
  await page.locator('#bulk-export-patients [role=option]').first().waitFor({ timeout: 15000 }).catch(() => {});
  await page.waitForTimeout(800);
  const opts = () => page.locator('#bulk-export-patients [role=option]').count();
  const footer = async () => ((await page.locator('#bulk-export-patients [data-combobox-footer]').first().textContent().catch(() => '')) || '').trim();
  const first = await opts();
  const f1 = await footer();
  const lb = page.locator('#bulk-export-patients [role=listbox]');
  let second = first, f2 = f1;
  if (/scroll for more/.test(f1)) {
    await lb.evaluate((el) => { el.scrollTop = el.scrollHeight; });
    await page.waitForFunction((n) => document.querySelectorAll('#bulk-export-patients [role=option]').length > n, first, { timeout: 15000 }).catch(() => {});
    await page.waitForTimeout(500);
    second = await opts(); f2 = await footer();
  }
  return { first, f1, second, f2 };
}
await row('5.2-picker', async () => {
  const apiN = async (t) => { const a = await api(`/Patient?name=${t}&_summary=count`, { headers: { Accept: 'application/fhir+json' } }); const b = await api(`/Patient?identifier=${t}&_summary=count`, { headers: { Accept: 'application/fhir+json' } }); return { name: a.json?.total, ident: b.json?.total }; };
  const pk = await pickerProbe(process.env.PK_TERM || 'Parker433');
  await shot(page, S, '5.2d-picker-parker433');
  const apiPk = await apiN(process.env.PK_TERM || 'Parker433');
  const ca = await pickerProbe('Cari853');
  await shot(page, S, '5.2e-picker-cari853');
  const apiCa = await apiN('Cari853');
  const m = /(\d+) matches/.exec(pk.f1);
  let ok, note = `Parker433: first page ${pk.first} options, footer "${pk.f1}", after scroll ${pk.second} options, footer "${pk.f2}" (API name total ${apiPk.name}); Cari853: ${ca.first} options, footer "${ca.f1}" (API name total ${apiCa.name})`;
  if (apiPk.name > 8) ok = pk.first === 8 && m && Number(m[1]) === apiPk.name && pk.second === Math.min(16, apiPk.name);
  else { ok = pk.first === apiPk.name; note += '; fewer than 9 Parker433 matches in this data so paging is not exercised'; }
  ok = ok && ca.first === apiCa.name;
  if (full) { const m38 = m && Number(m[1]) === 38 && pk.first === 8 && pk.second === 16 && ca.first === 3; ok = ok && !!m38; note += `; matrix (38 matches, 8 then 16 rows; Cari853 3): ${!!m38}`; }
  record(S, '5.2-picker', ok ? 'pass' : 'fail', note);
});

// ---------- 5.1 ----------
await row('5.1', async () => {
  const id = await startExport({ rowId: '5.1', name: 'everything-small', scope: 'system', types: ['Organization', 'Practitioner', 'Location'] });
  ids['5.1'] = id;
  const st = await waitDone(id);
  await showCard(id, '5.1b-card');
  const d = await files(id);
  await showDetail(id, '5.1c-files');
  const got = await fetchAll(d.files);
  const exp = {};
  for (const t of ['Organization', 'Practitioner', 'Location']) exp[t] = await total(t);
  const expFiles = Object.values(exp).reduce((a, n) => a + chunks(n), 0);
  const perType = {};
  for (const f of d.files) perType[f.type] = (perType[f.type] || 0) + got[f.label].lines.length;
  const wellFormed = Object.values(got).every((g) => g.lines.every((l) => { try { return typeof JSON.parse(l) === 'object'; } catch { return false; } }));
  let ok = st.status === 'complete' && d.files.length === expFiles && Object.keys(exp).every((t) => perType[t] === exp[t]) && wellFormed;
  let note = `status=${st.status}; card="${/(\d+ files?)/.exec(st.text)?.[1]}"; files=${d.files.length} (server-count expects ${expFiles}); lines ${JSON.stringify(perType)} vs stored ${JSON.stringify(exp)}; wellFormed=${wellFormed}`;
  if (full) {
    const m = d.files.length === 6 && perType.Organization === counts.Organization + 4 && perType.Location === counts.Location + 5 && perType.Practitioner === counts.Practitioner + 4;
    note += `; matrix(6 files, Org ${counts.Organization + 4}, Loc ${counts.Location + 5}, Pract ${counts.Practitioner + 4}; corpus + T2 fixtures): ${m}`;
    ok = ok && m;
  }
  const names = d.files.map((f) => f.label);
  if (full) { const n1 = names.includes('Location-0001.ndjson') && names.includes('Location-0002.ndjson'); ok = ok && n1; note += `; names Location-0001/0002=${n1}`; }
  record(S, '5.1', ok ? 'pass' : 'fail', note);
  // sample download for the evidence bundle
  for (const f of d.files.filter((x) => x.type === 'Organization')) fs.writeFileSync(path.join(OUTPUTS, `t5-5.1-${f.label}`), got[f.label].text);
  // ZIP
  await row('5.1-zip', async () => {
    await page.goto(`${BASE}/ui/bulk-export/active/${id}`);
    const [dl] = await Promise.all([page.waitForEvent('download'), page.getByRole('link', { name: /Download All Resources/ }).or(page.getByText('Download All Resources')).first().click()]);
    const zp = path.join(OUTPUTS, 't5-5.1-everything-small.zip');
    await dl.saveAs(zp);
    const buf = fs.readFileSync(zp);
    let entries = 0; for (let i = 0; i + 4 < buf.length; i++) if (buf[i] === 0x50 && buf[i + 1] === 0x4b && buf[i + 2] === 1 && buf[i + 3] === 2) entries++;
    const magic = buf.subarray(0, 2).toString();
    record(S, '5.1-zip', magic === 'PK' && entries === d.files.length ? 'pass' : 'fail', `suggested=${dl.suggestedFilename()} bytes=${buf.length} magic=${magic} entries=${entries} (expected ${d.files.length} NDJSON files)`);
  });
});

// ---------- 5.2 ----------
await row('5.2', async () => {
  const id = await startExport({ rowId: '5.2', name: 'one-patient', scope: 'patient', patientText: PID_QUERY, patientPick: process.env.PID_PICK || `Patient/${PID}`, patientRaw: `Patient/${PID}`, types: ['Patient', 'Condition', 'Observation'] });
  const st = await waitDone(id);
  await showCard(id, '5.2b-card');
  const d = await files(id);
  await showDetail(id, '5.2c-files');
  const got = await fetchAll(d.files);
  const n = {}; for (const f of d.files) n[f.type] = (n[f.type] || 0) + got[f.label].lines.length;
  const exp = { Patient: 1, Condition: await total('Condition', `patient=${PID}`), Observation: await total('Observation', `patient=${PID}`) };
  const patientLine = got[d.files.find((f) => f.type === 'Patient')?.label]?.lines[0];
  const pidOk = patientLine ? JSON.parse(patientLine).id === PID : false;
  let ok = st.status === 'complete' && d.files.length === 3 && n.Patient === 1 && n.Condition === exp.Condition && n.Observation === exp.Observation && pidOk;
  let note = `status=${st.status}; files=${d.files.length} (expect 3); lines ${JSON.stringify(n)} vs stored-by-API ${JSON.stringify(exp)}; Patient id==PID ${pidOk}`;
  if (full && PID === '7d24f7a0-6f2e-ce3b-5568-db7b14695583') { const m = n.Condition === 15 && n.Observation === 165; ok = ok && m; note += `; matrix(1/15/165): ${m}`; }
  record(S, '5.2', ok ? 'pass' : 'fail', note);
});

// ---------- 5.3 ----------
await row('5.3', async () => {
  const id = await startExport({ rowId: '5.3', name: 'group-active-conditions', scope: 'group', groupId: 'manual-group', types: ['Patient', 'Condition'], typeFilter: 'Condition?clinical-status=active', since: '' });
  const st = await waitDone(id);
  await showCard(id, '5.3b-card');
  const d = await files(id);
  await showDetail(id, '5.3c-files');
  const got = await fetchAll(d.files);
  const cond = d.files.filter((f) => f.type === 'Condition').flatMap((f) => got[f.label].lines.map((l) => JSON.parse(l)));
  const pat = d.files.filter((f) => f.type === 'Patient').flatMap((f) => got[f.label].lines);
  const allActive = cond.every((c) => (c.clinicalStatus?.coding || []).some((x) => x.code === 'active'));
  const allPid = cond.every((c) => c.subject?.reference === `Patient/${PID}`);
  const total5_2 = await total('Condition', `patient=${PID}`);
  const apiActive = await total('Condition', `patient=${PID}&clinical-status=active`);
  const ok = st.status === 'complete' && d.files.length === 2 && pat.length === 1 && cond.length === apiActive && allActive && allPid && (cond.length < total5_2 || total5_2 === 0 || apiActive === total5_2);
  record(S, '5.3', ok ? 'pass' : 'fail', `status=${st.status}; files=${d.files.length} (expect 2); Patient lines=${pat.length}; Condition lines=${cond.length} (API active=${apiActive}, all conditions=${total5_2}); allActive=${allActive} allBelongToPID=${allPid}`);
});

// ---------- 5.4 ----------
await row('5.4', async () => {
  const id = await startExport({ rowId: '5.4', name: 'elements-subset', scope: 'system', types: ['Patient'], elements: 'id,gender' });
  const st = await waitDone(id);
  await showCard(id, '5.4b-card');
  const d = await files(id);
  await showDetail(id, '5.4c-files');
  const got = await fetchAll(d.files);
  const all = d.files.flatMap((f) => got[f.label].lines.map((l) => JSON.parse(l)));
  const keysOk = all.every((p) => Object.keys(p).every((k) => ['resourceType', 'id', 'gender', 'meta'].includes(k)));
  const tagOk = all.every((p) => (p.meta?.tag || []).some((t) => t.code === 'SUBSETTED'));
  const nP = await total('Patient');
  const expFiles = chunks(nP);
  let ok = st.status === 'complete' && d.files.length === expFiles && all.length === nP && keysOk && tagOk;
  let note = `status=${st.status}; files=${d.files.length} (stored ${nP} Patients -> expect ${expFiles}); lines=${all.length}; only id/gender/meta keys=${keysOk}; meta.tag SUBSETTED on all=${tagOk}`;
  if (full) { const m = d.files.length === 12; ok = ok && m; note += `; matrix 12 files: ${m}`; }
  record(S, '5.4', ok ? 'pass' : 'fail', note);
});

// ---------- 5.5 cancel ----------
let cancelId;
await row('5.5', async () => {
  const id = await startExport({ rowId: '5.5', name: 'cancel-me', scope: 'system' });
  cancelId = id;
  const before = await cardState(id);
  if (before.status !== 'in-progress') {
    await showCard(id, '5.5b-already-done');
    record(S, '5.5', 'na', `export finished before Cancel could be clicked (status=${before.status}); scratch data too small - on the full corpus this runs for a long time`);
    return;
  }
  await page.goto(`${BASE}/ui/bulk-export`);
  await page.waitForTimeout(500);
  await shot(page, S, '5.5b-in-progress');
  await page.locator(`#job-${id} form[action$="/cancel"] button`).click();
  await page.waitForTimeout(1500);
  await page.goto(`${BASE}/ui/bulk-export`);
  const s = await cardState(id);
  await shot(page, S, '5.5c-cancelled');
  record(S, '5.5', s.status === 'cancelled' ? 'pass' : 'fail', `after Cancel chip status=${s.status} text="${s.text.slice(0, 120)}" (expected cancelled)`);
});

// ---------- delete cancel-me ----------
async function deleteCard(id, label) {
  await page.goto(`${BASE}/ui/bulk-export`);
  const c = page.locator(`#job-${id}`);
  await c.locator('details.menu > summary').click();
  await c.locator('details.job-card__delete > summary').click();
  await page.waitForTimeout(400);
  const warn = await page.getByText(/This cannot be undone/).first().textContent().catch(() => '');
  await shot(page, S, `${label}-delete-confirm`);
  await page.getByRole('button', { name: 'Delete export' }).first().click();
  await page.waitForTimeout(1200);
  await page.goto(`${BASE}/ui/bulk-export`);
  const gone = (await page.locator(`#job-${id}`).count()) === 0;
  return { gone, warn: (warn || '').trim() };
}
await row('5.5-delete', async () => {
  if (!cancelId) { record(S, '5.5-delete', 'na', 'cancel-me card was not created'); return; }
  const { gone, warn } = await deleteCard(cancelId, '5.5d');
  const ok = gone && /Delete cancel-me and its output files from the server\? This cannot be undone\./.test(warn);
  record(S, '5.5-delete', ok ? 'pass' : 'fail', `card removed=${gone}; warning="${warn}"`);
});

// ---------- window helpers ----------
async function instantsFallback() {
  let start = process.env.T3_START, end = process.env.T3_END;
  if (!start) {
    const r = await api('/Patient?_sort=_lastUpdated&_count=2', { headers: { Accept: 'application/fhir+json' } });
    const t = (r.json?.entry || []).map((e) => e.resource?.meta?.lastUpdated).filter(Boolean);
    if (t.length === 2) start = new Date((Date.parse(t[0]) + Date.parse(t[1])) / 2).toISOString().replace(/\.\d+Z$/, 'Z');
  }
  if (!end) {
    const r = await api('/RiskAssessment/manual-risk', { headers: { Accept: 'application/fhir+json' } });
    const lu = r.json?.meta?.lastUpdated;
    if (lu) end = new Date(Date.parse(lu) - 1000).toISOString().replace(/\.\d+Z$/, 'Z');
  }
  return { start, end };
}
const { start: T3S, end: T3E } = await instantsFallback();
console.log(`T3_START=${T3S} T3_END=${T3E}`);

async function windowExport(rid, opts, wantWindow, expectPatient, expectFiles) {
  const id = await startExport({ rowId: rid, ...opts });
  const st = await waitDone(id);
  await showCard(id, `${rid}b-card`);
  const d = await files(id);
  await showDetail(id, `${rid}c-files`);
  const got = await fetchAll(d.files);
  const n = {}; for (const f of d.files) n[f.type] = (n[f.type] || 0) + got[f.label].lines.length;
  return { id, st, d, got, n, windowOk: wantWindow ? wantWindow.every((w) => st.text.includes(w)) : true };
}

// ---------- 5.7 ----------
await row('5.7', async () => {
  if (!T3S) return record(S, '5.7', 'na', 'T3_START not given and not derivable');
  const r = await windowExport('5.7', { name: 'since-import', scope: 'system', types: ['Patient'], since: 'custom', sinceCustom: T3S }, ['Since', T3S], 0);
  const exp = await total('Patient', `_lastUpdated=ge${T3S}`);
  let ok = r.st.status === 'complete' && (r.n.Patient || 0) === exp && r.windowOk && r.d.files.length === chunks(exp);
  let note = `status=${r.st.status}; Patient lines=${r.n.Patient || 0} vs API _lastUpdated>=T3_START ${exp}; files=${r.d.files.length}; window line has "Since ${T3S}"=${r.windowOk}`;
  if (full) { const m = (r.n.Patient || 0) === counts.Patient && r.d.files.length === 12; ok = ok && m; note += `; matrix ${counts.Patient} lines/12 files: ${m}`; }
  record(S, '5.7', ok ? 'pass' : 'fail', note);
});
// ---------- 5.8 ----------
await row('5.8', async () => {
  if (!T3S) return record(S, '5.8', 'na', 'T3_START not available');
  const r = await windowExport('5.8', { name: 'until-import', scope: 'system', types: ['Patient'], since: '', until: T3S }, ['Until', T3S]);
  const exp = await total('Patient', `_lastUpdated=le${T3S}`);
  let ok = r.st.status === 'complete' && (r.n.Patient || 0) === exp && r.windowOk;
  let note = `status=${r.st.status}; Patient lines=${r.n.Patient || 0} vs API _lastUpdated<=T3_START ${exp}; window "Until ${T3S}"=${r.windowOk}`;
  const fam = r.d.files.length ? r.got[r.d.files[0].label].lines.map((l) => JSON.parse(l).name?.[0]?.family) : [];
  note += `; families=${JSON.stringify(fam.slice(0, 3))}`;
  if (full) { const m = (r.n.Patient || 0) === 1 && /Larkin917/.test(fam[0] || ''); ok = ok && m; note += `; matrix 1 Larkin917: ${m}`; }
  record(S, '5.8', ok ? 'pass' : 'fail', note);
});
// ---------- 5.9 ----------
await row('5.9', async () => {
  if (!T3S || !T3E) return record(S, '5.9', 'na', 'T3_START/T3_END not available');
  const r = await windowExport('5.9', { name: 'since-until', scope: 'system', types: ['Patient'], since: 'custom', sinceCustom: T3S, until: T3E }, [T3S, T3E]);
  const exp = await total('Patient', `_lastUpdated=ge${T3S}&_lastUpdated=le${T3E}`);
  let ok = r.st.status === 'complete' && (r.n.Patient || 0) === exp && r.windowOk;
  let note = `status=${r.st.status}; Patient lines=${r.n.Patient || 0} vs API window ${exp}; window line has both instants=${r.windowOk}; text="${r.st.text.slice(0, 160)}"`;
  if (full) { const m = (r.n.Patient || 0) === counts.Patient; ok = ok && m; note += `; matrix ${counts.Patient}: ${m}`; }
  record(S, '5.9', ok ? 'pass' : 'fail', note);
});
// ---------- 5.10 ----------
await row('5.10', async () => {
  if (!T3E) return record(S, '5.10', 'na', 'T3_END not available (no RiskAssessment manual-risk to derive it from)');
  const r = await windowExport('5.10', { name: 'since-fixtures', scope: 'system', types: ['Patient', 'RiskAssessment', 'ValueSet', 'Group'], since: 'custom', sinceCustom: T3E }, ['Since', T3E]);
  const types = r.d.files.map((f) => f.type);
  const ids = {};
  for (const f of r.d.files) ids[f.type] = r.got[f.label].lines.map((l) => JSON.parse(l).id);
  const exp = { RiskAssessment: await total('RiskAssessment', `_lastUpdated=ge${T3E}`), ValueSet: await total('ValueSet', `_lastUpdated=ge${T3E}`), Group: await total('Group', `_lastUpdated=ge${T3E}`) };
  const nPat = r.n.Patient || 0;
  const ok = r.st.status === 'complete' && ['RiskAssessment', 'ValueSet', 'Group'].every((t) => (r.n[t] || 0) === exp[t]) && nPat === (await total('Patient', `_lastUpdated=ge${T3E}`)) && r.windowOk;
  let note = `status=${r.st.status}; lines ${JSON.stringify(r.n)} vs API ${JSON.stringify(exp)}; ids ${JSON.stringify(ids)}; Patient pill ${types.includes('Patient') ? 'present' : 'absent'}`;
  const m = (r.n.RiskAssessment === 1 && r.n.ValueSet === 1 && r.n.Group === 1 && nPat === 0);
  note += `; matrix 1/1/1 and no Patient: ${m}`;
  record(S, '5.10', (full ? ok && m : ok) ? 'pass' : 'fail', note);
});
// ---------- 5.11 ----------
await row('5.11', async () => {
  const r = await windowExport('5.11', { name: 'last-day', scope: 'system', types: ['Organization'], since: 'day' }, ['Since']);
  const dayAgo = new Date(Date.now() - 86400000).toISOString().replace(/\.\d+Z$/, 'Z');
  const exp = await total('Organization', `_lastUpdated=ge${dayAgo}`);
  const inst = (r.st.text.match(/Since\s+(\S+)/) || [])[1];
  const skew = inst ? Math.abs(Date.parse(inst) - (Date.now() - 86400000)) / 60000 : NaN;
  const ok = r.st.status === 'complete' && (r.n.Organization || 0) === exp && skew < 30 && (exp > 0 || r.d.files.length === 0 || (r.n.Organization || 0) === 0);
  record(S, '5.11', ok ? 'pass' : 'fail', `status=${r.st.status}; window instant=${inst} (${isNaN(skew) ? '?' : skew.toFixed(1)} min from now-24h); Organization lines=${r.n.Organization || 0} vs API last-24h ${exp}; files=${r.d.files.length}; card="${/(\d+ files?)/.exec(r.st.text)?.[1]}"`);
});
// ---------- 5.12 ----------
await row('5.12', async () => {
  await fillForm({ name: 'bad-since', scope: 'system', types: ['Organization'], since: 'custom', sinceCustom: 'yesterday' });
  await submit();
  await page.waitForTimeout(700);
  const err = page.locator('#bulk-export-since-custom-error');
  const vis = await err.isVisible().catch(() => false);
  const txt = (await err.textContent().catch(() => '') || '').trim();
  await shot(page, S, '5.12a-invalid-instant');
  const stay = /\/ui\/bulk-export\/new/.test(page.url());
  const first = vis && stay && txt === 'Enter a valid FHIR instant, such as 2026-08-01T00:00:00Z.';
  // preset back to All time -> the field is disabled and no longer blocks
  await pickFixed('bulk-export-since', 'since_preset', '', 'All time');
  await page.waitForTimeout(300);
  const disabled = await page.locator('input[name=since_custom]').isDisabled();
  const before = new Set(await (async () => { await page.goto(`${BASE}/ui/bulk-export`); return cardIds(); })());
  await fillForm({ name: 'since-reset', scope: 'system', types: ['Organization'], since: 'custom', sinceCustom: 'yesterday' });
  await pickFixed('bulk-export-since', 'since_preset', '', 'All time');
  await page.waitForTimeout(300);
  const dis2 = await page.locator('input[name=since_custom]').isDisabled();
  await shot(page, S, '5.12b-preset-reset');
  await submit();
  let started = false, newId;
  try { await page.waitForURL(/\/ui\/bulk-export(\?.*)?$/, { timeout: 15000 }); const now = await cardIds(); const f = now.filter((i) => !before.has(i)); started = f.length === 1; newId = f[0]; } catch {}
  record(S, '5.12', first && dis2 && started ? 'pass' : 'fail', `error visible=${vis} text="${txt}" stayedOnForm=${stay}; after preset->All time field disabled=${dis2}, submit started an export=${started}`);
  // no-JS: server 400 on impossible instant
  const ctx = await browser.newContext({ javaScriptEnabled: false });
  const p2 = await ctx.newPage();
  await p2.goto(`${BASE}/ui/bulk-export/new`);
  await p2.fill('input[name=name]', 'nojs-bad-since');
  await p2.locator('input[name=scope][value=system]').check();
  await p2.evaluate(() => { const s = document.querySelector('select[name=since_preset]'); s.value = 'custom'; });
  await p2.fill('input[name=since_custom]', 'yesterday').catch(async () => { await p2.evaluate(() => { const i = document.querySelector('input[name=since_custom]'); i.disabled = false; i.value = 'yesterday'; }); });
  const [r2] = await Promise.all([p2.waitForResponse((r) => r.request().method() === 'POST'), p2.locator('form.bulk-export-form button[type=submit]').click()]);
  const body = await p2.content();
  record(S, '5.12-nojs', r2.status() === 400 && body.includes('Enter a valid FHIR instant') ? 'pass' : 'fail', `no-JS POST -> ${r2.status()}, message present=${body.includes('Enter a valid FHIR instant')}`);
  await ctx.close();
  if (newId) { await waitDone(newId); await deleteCard(newId, '5.12c').catch(() => {}); }
});

// ---------- 5.13 ----------
await row('5.13', async () => {
  const id = await startExport({ rowId: '5.13', name: 'bad-group', scope: 'group', groupId: 'does-not-exist', types: ['Patient'] });
  const st = await waitDone(id);
  await showCard(id, '5.13b-failed');
  const named = /does-not-exist/.test(st.text);
  // retry
  const before = new Set(await cardIds());
  await page.locator(`#job-${id} form[action$="/retry"] button`).click();
  await page.waitForTimeout(1500);
  await page.goto(`${BASE}/ui/bulk-export`);
  const now = await cardIds();
  const fresh = now.filter((i) => !before.has(i));
  let st2 = { status: '?', text: '' };
  if (fresh.length === 1) st2 = await waitDone(fresh[0]);
  await shot(page, S, '5.13c-retry');
  const stillThere = now.includes(id);
  const ok = st.status === 'failed' && named && fresh.length === 1 && st2.status === 'failed' && stillThere;
  record(S, '5.13', ok ? 'pass' : 'fail', `first card status=${st.status} text="${st.text.slice(0, 200)}" names missing Group=${named}; retry created ${fresh.length} new card, status=${st2.status}, original card kept=${stillThere}`);
  const d1 = await deleteCard(id, '5.13d');
  let d2 = { gone: true };
  if (fresh[0]) d2 = await deleteCard(fresh[0], '5.13e');
  record(S, '5.13-delete', d1.gone && d2.gone ? 'pass' : 'fail', `both cards deleted: first=${d1.gone} retry=${d2.gone}`);
});

// ---------- API cross-checks ----------
const PREFER = { Prefer: 'respond-async', Accept: 'application/fhir+json' };
async function kick(urlPath, extra = {}) {
  return api(urlPath, { headers: { ...PREFER, ...extra } });
}
async function pollManifest(statusUrl, save) {
  const t0 = Date.now();
  for (;;) {
    const r = await api(statusUrl, { headers: { Accept: 'application/fhir+json' } });
    if (r.status === 200) { if (save) await api(statusUrl, { headers: { Accept: 'application/fhir+json' }, save }); return r; }
    if (r.status !== 202 && r.status !== 303) return r;
    if (Date.now() - t0 > TMO) return r;
    await sleep(2000);
  }
}
async function apiExport(rid, label, urlPath, saveName, check) {
  await row(rid, async () => {
    const k = await kick(urlPath);
    const loc = k.headers['content-location'];
    if (k.status !== 202 || !loc) return record(S, rid, 'fail', `${label}: kick-off ${k.status} ${k.text.slice(0, 200)} (expected 202 + Content-Location)`);
    const m = await pollManifest(loc, saveName);
    if (m.status !== 200) return record(S, rid, 'fail', `${label}: status poll ended ${m.status} ${m.text.slice(0, 200)}`);
    const man = m.json;
    const outs = man.output || [];
    const res = await check(man, outs);
    record(S, rid, res.ok ? 'pass' : 'fail', `${label}: manifest ${outs.length} file(s) [${outs.map((o) => `${o.type}:${o.count}`).join(', ')}] requiresAccessToken=${man.requiresAccessToken}; ${res.note}`);
  });
}
async function downloadCount(o) {
  const r = await api(o.url);
  return { status: r.status, n: lines(r.text).length, ct: r.headers['content-type'] };
}
await apiExport('5.A1-system', 'system $export _type=Organization', `/$export?_type=Organization`, 't5-manifest-system.json', async (man, outs) => {
  const exp = await total('Organization');
  let got = 0; let ct = '';
  for (const o of outs.filter((x) => x.type === 'Organization')) { const d = await downloadCount(o); got += d.n; ct = d.ct; }
  return { ok: got === exp && outs.every((o) => o.type === 'Organization'), note: `downloaded ${got} lines vs stored ${exp}; content-type=${ct}` };
});
await apiExport('5.A2-patient', 'patient $export _type=Patient,Condition', `/Patient/$export?_type=Patient,Condition`, 't5-manifest-patient.json', async (man, outs) => {
  const eP = await total('Patient'), eC = await total('Condition');
  let p = 0, c = 0;
  for (const o of outs) { const d = await downloadCount(o); if (o.type === 'Patient') p += d.n; if (o.type === 'Condition') c += d.n; }
  return { ok: p === eP && c === eC, note: `Patient ${p}/${eP}, Condition ${c}/${eC}` };
});
await apiExport('5.A3-group', 'group $export manual-group _type=Patient,Condition', `/Group/manual-group/$export?_type=Patient,Condition`, 't5-manifest-group.json', async (man, outs) => {
  let p = 0, c = 0, allPid = true;
  for (const o of outs) {
    const r = await api(o.url);
    const ls = lines(r.text).map((l) => JSON.parse(l));
    if (o.type === 'Patient') p += ls.length;
    if (o.type === 'Condition') { c += ls.length; allPid = allPid && ls.every((x) => x.subject?.reference === `Patient/${PID}`); }
  }
  const eC = await total('Condition', `patient=${PID}`);
  return { ok: p === 1 && c === eC && allPid, note: `Patient ${p} (expect 1), Condition ${c} vs stored ${eC}, all belong to PID=${allPid}` };
});

// three NDJSON spellings
for (const fmt of ['application/fhir+ndjson', 'application/ndjson', 'ndjson']) {
  await row(`5.A4-format-${fmt}`, async () => {
    const k = await kick(`/$export?_type=Organization&_outputFormat=${encodeURIComponent(fmt)}`);
    const loc = k.headers['content-location'];
    let note = `kick-off ${k.status}`;
    let ok = k.status === 202 && !!loc;
    if (ok) {
      const m = await pollManifest(loc);
      const o = m.json?.output?.[0];
      ok = m.status === 200 && !!o;
      note += `; manifest ${m.status}, first file ${o?.type}:${o?.count}`;
      if (o) { const d = await downloadCount(o); note += `; downloaded ${d.n} lines, content-type ${d.ct}`; ok = ok && d.n > 0 === ((await total('Organization')) > 0); }
    } else note += ` ${k.text.slice(0, 160)}`;
    record(S, `5.A4-format-${fmt}`, ok ? 'pass' : 'fail', `_outputFormat=${fmt}: ${note} (expected 202 and a complete NDJSON manifest)`);
  });
}
// unsupported formats
for (const fmt of ['application/fhir+json', 'text/csv', 'parquet']) {
  await row(`5.A5-bad-format-${fmt}`, async () => {
    const k = await kick(`/$export?_type=Organization&_outputFormat=${encodeURIComponent(fmt)}`);
    const oo = k.json;
    const ok = k.status === 400 && oo?.resourceType === 'OperationOutcome' && /unsupported _outputFormat/.test(JSON.stringify(oo));
    record(S, `5.A5-bad-format-${fmt}`, ok ? 'pass' : 'fail', `_outputFormat=${fmt} -> ${k.status} ${oo?.issue?.[0]?.diagnostics || k.text.slice(0, 160)} (expected 400 OperationOutcome "unsupported _outputFormat '${fmt}'")`);
  });
}
await row('5.A5-no-prefer', async () => {
  const k = await api('/$export?_type=Organization', { headers: { Accept: 'application/fhir+json' } });
  record(S, '5.A5-no-prefer', k.status === 400 ? 'pass' : 'fail', `no Prefer: respond-async -> ${k.status} ${k.json?.issue?.[0]?.diagnostics || ''}`);
});
await row('5.A5-until-before-since', async () => {
  const k = await kick('/$export?_type=Organization&_since=2026-02-01T00:00:00Z&_until=2026-01-01T00:00:00Z');
  record(S, '5.A5-until-before-since', k.status === 400 ? 'pass' : 'fail', `_until < _since -> ${k.status} ${k.json?.issue?.[0]?.diagnostics || ''} (expected 400)`);
});
await row('5.A6-bad-group', async () => {
  const k = await kick('/Group/does-not-exist/$export?_type=Patient');
  let note = `kick-off ${k.status} ${k.json?.issue?.[0]?.diagnostics || ''}`;
  let ok = k.status === 404 || k.status === 400;
  if (k.status === 202) {
    const r = await pollManifest(k.headers['content-location']);
    note += `; accepted then status ${r.status} ${r.json?.issue?.[0]?.diagnostics || r.text.slice(0, 120)}`;
    ok = r.status >= 400;
  }
  record(S, '5.A6-bad-group', ok ? 'pass' : 'fail', `${note} (the UI row 5.13 shows Failed; the API accepted-or-rejected outcome is recorded here)`);
});
// API _since/_until window
await row('5.A7-since-window', async () => {
  const since = T3S || '2000-01-01T00:00:00Z';
  const k = await kick(`/$export?_type=Patient&_since=${encodeURIComponent(since)}&_until=${encodeURIComponent(T3E || '2999-01-01T00:00:00Z')}`);
  const m = await pollManifest(k.headers['content-location']);
  let n = 0; for (const o of m.json?.output || []) n += (await downloadCount(o)).n;
  const exp = await total('Patient', `_lastUpdated=ge${since}&_lastUpdated=le${T3E || '2999-01-01T00:00:00Z'}`);
  record(S, '5.A7-since-window', n === exp ? 'pass' : 'fail', `window ${since}..${T3E || 'open'}: ${n} Patient lines vs API ${exp}`);
});
// API cancel
await row('5.A8-cancel', async () => {
  if (process.env.SKIP_CANCEL_ALL === '1') return record(S, '5.A8-cancel', 'na', 'skipped by SKIP_CANCEL_ALL');
  const k = await kick('/$export');
  const loc = k.headers['content-location'];
  if (k.status !== 202 || !loc) return record(S, '5.A8-cancel', 'fail', `kick-off ${k.status}`);
  const del = await api(loc, { method: 'DELETE', headers: { Accept: 'application/fhir+json' } });
  await sleep(1000);
  const after = await api(loc, { headers: { Accept: 'application/fhir+json' } });
  record(S, '5.A8-cancel', del.status === 202 && after.status === 404 ? 'pass' : 'fail', `DELETE status -> ${del.status} (expect 202); GET after -> ${after.status} (expect 404)`);
});

await browser.close();
console.log('T5 done');
