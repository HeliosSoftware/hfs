// T2 - Batch / Transaction page (matrix section 6). Runs on an EMPTY server straight after T1, before the corpus import.
// Rows: 6.1 (negative transaction), 6.2a (hospital batch), 6.2b (practitioner batch), 6.2c (rail + identifier search),
//       6.3a (per-action outcomes of the transaction), 6.3b (patient + counts), 6.3c (rewritten references),
//       6.4a (not-a-bundle), 6.4b (collection), 6.4c (non-JSON).
// Env: HFS_BASE, OUT (see lib.mjs); FIXTURES (default /home/edson/tmp/939/work/fixtures: the three files of fhir-batch-import.tar.gz);
//      BACKEND (default pg-es; s3/s3-es => 6.1 and 6.3* na).
// Writes the matrix 6.4 fixtures (not-a-bundle.json, collection.json, not-json.log) to $OUT/outputs/t2-fixtures/.
import fs from 'fs';
import path from 'path';
import { launch, shot, record, api, BASE, OUTPUTS } from '../lib.mjs';

const S = 'T2';
const BACKEND = process.env.BACKEND || 'pg-es';
const S3 = /^s3/.test(BACKEND);
const FIX = process.env.FIXTURES || '/home/edson/tmp/939/work/fixtures';
const FH = { Accept: 'application/fhir+json' };
const find = (re) => { const f = fs.readdirSync(FIX).find((n) => re.test(n)); if (!f) throw new Error(`fixture ${re} not in ${FIX}`); return path.join(FIX, f); };
const F_HOSP = find(/^hospitalInformation.*\.json$/), F_PRAC = find(/^practitionerInformation.*\.json$/), F_PAT = find(/^Nicky270_.*\.json$/);
const FX = path.join(OUTPUTS, 't2-fixtures'); fs.mkdirSync(FX, { recursive: true });
fs.writeFileSync(path.join(FX, 'not-a-bundle.json'), '{"resourceType":"Patient","id":"x"}');
fs.writeFileSync(path.join(FX, 'collection.json'), '{"resourceType":"Bundle","type":"collection","entry":[]}');
fs.writeFileSync(path.join(FX, 'not-json.log'), 'build started\nthis is not JSON at all\n');
const bundle = (f) => JSON.parse(fs.readFileSync(f, 'utf8'));
const tryRow = async (id, fn) => { try { await fn(); } catch (e) { record(S, id, 'fail', `script error: ${String(e.message || e).split('\n')[0]}`); } };
const rec = (id, subs) => {
  const bad = subs.filter((s) => !s.ok);
  record(S, id, bad.length ? 'fail' : 'pass', subs.map((s) => `${s.ok ? 'ok' : 'FAIL'}: ${s.msg}`).join(' | '));
};
const sub = (ok, msg) => ({ ok: !!ok, msg });
const count = async (type, q = '') => (await api(`/${type}?${q}${q ? '&' : ''}_summary=count`, { headers: FH })).json?.total;
const txt = async (loc) => ((await loc.textContent().catch(() => '')) || '').replace(/\s+/g, ' ').trim();

const { browser, page } = await launch();
page.setDefaultTimeout(20000);

// ---------- page helpers ----------
async function gotoBatch() { await page.goto(`${BASE}/ui/batch`); await page.locator('#batch-drop').waitFor(); }
// pick a file; resolves to 'preflight' or 'error' (stays on Upload)
async function upload(file) {
  await page.locator('#batch-file').setInputFiles(file);
  await page.waitForFunction(() => !document.getElementById('batch-preflight').hidden || (!document.getElementById('batch-upload-error').hidden && document.getElementById('batch-upload-error').textContent.trim()), null, { timeout: 60000 });
  return (await page.locator('#batch-preflight').isVisible()) ? 'preflight' : 'error';
}
async function plan() {
  return { line: await txt(page.locator('#batch-request-line')), notice: await txt(page.locator('#batch-semantics')), rows: await page.locator('#batch-rows > li').count() };
}
async function railCounts(types) {
  await page.goto(`${BASE}/ui/resources`);
  await page.locator('#type-rail-list').waitFor();
  const out = {};
  for (const t of types) {
    const t0 = await txt(page.locator(`.filter-rail__item[data-type="${t}"] .count`).first());
    const m = t0.match(/^(≈)?\s*([\d,]+)/);
    out[t] = m ? { n: Number(m[2].replace(/,/g, '')), approx: !!m[1], raw: t0 } : { n: null, raw: t0 };
  }
  return out;
}
const fmtRail = (r) => Object.entries(r).map(([k, v]) => `${k} ${v.approx ? '≈' : ''}${v.n}`).join(', ');
// query through the Resources query builder (screenshot) and return the observed totals
async function uiQuery(typed, shotName) {
  await page.goto(`${BASE}/ui/resources`);
  const input = page.locator('input.query-builder__url');
  await input.waitFor();
  const type = typed.replace(/^\//, '').split(/[?/]/)[0];
  const wait = page.waitForResponse((r) => r.request().method() === 'GET' && !r.url().includes('/ui/') && r.url().includes('/' + type) && r.url().startsWith(BASE), { timeout: 60000 }).catch(() => null);
  await input.fill(`GET ${typed}`);
  await input.press('Enter');
  await wait;
  let meta = '';
  for (let i = 0; i < 24; i++) { await page.waitForTimeout(250); meta = await txt(page.locator('#query-results-meta')); if (meta && !(await page.locator('#query-search-status').isVisible().catch(() => false))) break; }
  const m = meta.match(/([\d,]+)\+?\s+results?/);
  if (shotName) await shot(page, S, shotName);
  return { meta, total: m ? Number(m[1].replace(/,/g, '')) : null };
}
// execute and wait for the outcome stage or the error; optional delay (ms) on the POST so "Executing..." is observable
async function execute({ delay = 0 } = {}) {
  let busyOk = null;
  const handler = async (route) => {
    if (route.request().method() === 'POST') await new Promise((r) => setTimeout(r, delay));
    await route.continue();
  };
  const matcher = (u) => u.origin + u.pathname === BASE + '/';
  if (delay) await page.route(matcher, handler);
  await page.locator('#batch-execute-top').click();
  if (delay) {
    await page.waitForTimeout(300);
    const dis = await page.evaluate(() => ({ ex: document.getElementById('batch-execute-top').disabled, ca: document.getElementById('batch-cancel-top').disabled }));
    busyOk = { ...dis, label: await txt(page.locator('#batch-busy')) };
  }
  await page.waitForFunction(() => !document.getElementById('batch-response').hidden || (!document.getElementById('batch-execute-error').hidden && document.getElementById('batch-execute-error').textContent.trim()), null, { timeout: 300000 });
  if (delay) await page.unroute(matcher, handler).catch(() => {});
  return { resp: await page.locator('#batch-response').isVisible(), busyOk };
}
async function outcome() {
  const badges = await page.locator('#batch-outcomes > li .batch-badge').allTextContents();
  return { http: await txt(page.locator('#batch-overall')), created: await txt(page.locator('#batch-created')), summary: await txt(page.locator('#batch-summary')), badges: badges.map((b) => b.trim()) };
}
const allCreated = (o, n) => o.badges.length === n && o.badges.every((b) => /^201\b/.test(b));
const badgeSet = (o) => [...new Set(o.badges)].join(',');

// ---------- precondition: empty server ----------
const pre = { Patient: await count('Patient'), Organization: await count('Organization'), Practitioner: await count('Practitioner') };
console.log('precondition counts', JSON.stringify(pre));
if (!Object.values(pre).every((n) => n === 0)) {
  record(S, '6.1', 'fail', `server is NOT empty (${JSON.stringify(pre)}): T2 must run on the empty server straight after T1; script stopped without writing anything`);
  await browser.close(); process.exit(0);
}

const bHosp = bundle(F_HOSP), bPrac = bundle(F_PRAC);
await page.goto(`${BASE}/ui`); await page.waitForTimeout(2000);
const dashBefore = await txt(page.locator('#dash-chart .stat__value').first());

// ================= 6.1 =================
await tryRow('6.1', async () => {
  if (S3) return record(S, '6.1', 'na', 'S3 backend: the transaction is refused for a different reason (no multi-object atomicity)');
  const subs = [];
  await gotoBatch();
  const kind = await upload(F_PAT);
  subs.push(sub(kind === 'preflight', `upload of ${path.basename(F_PAT)} moved to ${kind === 'preflight' ? 'Execution Plan' : 'an error: ' + await txt(page.locator('#batch-upload-error'))}`));
  if (kind !== 'preflight') return rec('6.1', subs);
  const p = await plan();
  subs.push(sub(p.line === 'POST [base] · Bundle · transaction · 662 entries', `request strip "${p.line}" (expected "POST [base] · Bundle · transaction · 662 entries")`));
  subs.push(sub(p.notice === 'Transaction: all or nothing — if any entry fails, the server rolls the whole bundle back.', `notice "${p.notice}"`));
  subs.push(sub((await page.locator('#batch-preflight select').count()) === 0, 'no batch/transaction selector on the page'));
  const heads = page.locator('#batch-rows > li > .batch-row__head');
  const h1 = await txt(heads.nth(0)), h2 = await txt(heads.nth(1));
  subs.push(sub(/POST/.test(h1) && /Patient/.test(h1), `entry 1 reads "${h1}" (expected POST Patient)`));
  subs.push(sub(/POST/.test(h2) && /Encounter/.test(h2), `entry 2 reads "${h2}" (expected POST Encounter)`));
  await heads.nth(0).click(); await page.waitForTimeout(800);
  const body1 = await txt(page.locator('#batch-rows > li').nth(0).locator('.batch-row__body'));
  subs.push(sub(/Nicky270/.test(body1) && /Ann985/.test(body1) && /Larkin917/.test(body1) && /1996-04-19/.test(body1) && /Millis/.test(body1), `entry 1 body shows Nicky270/Ann985/Larkin917, 1996-04-19, Millis (${body1.length} chars)`));
  await shot(page, S, '6.1-plan-entry1-patient');
  await heads.nth(1).click(); await page.waitForTimeout(800);
  const body2 = await txt(page.locator('#batch-rows > li').nth(1).locator('.batch-row__body'));
  const sp = body2.match(/Organization\?identifier=https:\/\/github\.com\/synthetichealth\/synthea\|[0-9a-f-]+/);
  const pr = body2.match(/Practitioner\?identifier=http:\/\/hl7\.org\/fhir\/sid\/us-npi\|\d+/);
  subs.push(sub(sp && pr, `entry 2 body: serviceProvider.reference ${sp ? sp[0] : 'NOT FOUND'}; participant[0].individual.reference ${pr ? pr[0] : 'NOT FOUND'}`));
  await shot(page, S, '6.1-plan-entry2-conditional-refs');
  const ex = await execute({ delay: 1200 });
  subs.push(sub(ex.busyOk && ex.busyOk.ex && ex.busyOk.ca && /Executing/.test(ex.busyOk.label), `while running: Execute disabled=${ex.busyOk?.ex}, Cancel disabled=${ex.busyOk?.ca}, status "${ex.busyOk?.label}" (expected both disabled and "Executing…")`));
  const err = await txt(page.locator('#batch-execute-error'));
  const okMsg = /^The request failed\. — Conditional reference '(Organization|Practitioner|Location)\?identifier=[^']+' matches no existing resource$/.test(err);
  subs.push(sub(!ex.resp && okMsg && (await page.locator('#batch-preflight').isVisible()), `error above the plan (exact): "${err}" (expected "The request failed. — Conditional reference '<Organization|Practitioner|Location>?identifier=…' matches no existing resource"); stayed on Execution Plan=${await page.locator('#batch-preflight').isVisible()}`));
  await page.evaluate(() => window.scrollTo(0, 0));
  await shot(page, S, '6.1-execute-error');
  const rail = await railCounts(['Patient', 'Encounter', 'Observation']);
  const apiC = { Patient: await count('Patient'), Encounter: await count('Encounter'), Observation: await count('Observation'), Organization: await count('Organization'), Practitioner: await count('Practitioner') };
  await shot(page, S, '6.1-rollback-resources-rail');
  await page.goto(`${BASE}/ui`); await page.waitForTimeout(2000);
  await shot(page, S, '6.1-rollback-dashboard');
  const dash = await txt(page.locator('#dash-chart .stat__value').first());
  subs.push(sub(Object.values(rail).every((v) => v.n === 0) && Object.values(apiC).every((n) => n === 0), `rollback: rail ${fmtRail(rail)}; API counts ${JSON.stringify(apiC)} (all expected 0)`));
  subs.push(sub(dash === dashBefore, `dashboard chart total unchanged: "${dashBefore}" -> "${dash}"`));
  // step 8: Cancel with the error showing
  await gotoBatch(); await upload(F_PAT);
  await execute({});
  const errShown = await page.locator('#batch-execute-error').isVisible();
  await page.locator('#batch-cancel-top').click();
  await page.waitForTimeout(500);
  const upVis = await page.locator('#batch-upload').isVisible(), errVis = await page.locator('#batch-execute-error').isVisible();
  subs.push(sub(errShown && upVis && !errVis, `Cancel: error shown before=${errShown}; after: Upload stage visible=${upVis}, error visible=${errVis} (expected true/true/false)`));
  await shot(page, S, '6.1-cancel-upload-stage');
  rec('6.1', subs);
});

// ================= 6.2 =================
async function batchStep(id, file, b, nameShot) {
  const subs = [];
  const n = b.entry.length;
  await gotoBatch();
  const kind = await upload(file);
  if (kind !== 'preflight') return rec(id, [sub(false, `upload error: ${await txt(page.locator('#batch-upload-error'))}`)]);
  const p = await plan();
  subs.push(sub(p.line === `POST [base] · Bundle · batch · ${n} entries`, `strip "${p.line}" (expected "POST [base] · Bundle · batch · ${n} entries")`));
  subs.push(sub(p.notice === 'Batch: entries run independently — a failed entry does not stop or undo the others.', `notice "${p.notice}"`));
  const chips = (await page.locator('#batch-rows > li .batch-chip').allTextContents()).map((s) => s.trim());
  const urls = (await page.locator('#batch-rows > li .batch-row__url').allTextContents()).map((s) => s.trim());
  const got = urls.map((u, i) => `${chips[i]} ${u}`);
  const want = b.entry.map((e) => `${e.request.method} ${e.request.url}`);
  subs.push(sub(got.length === n && got.every((g, i) => g === want[i]), `Actions follow file order: ${got.join(', ')}`));
  await shot(page, S, `${nameShot}-plan`);
  await execute({});
  const o = await outcome();
  subs.push(sub(o.http === '200' && o.created === `${n} created` && allCreated(o, n), `Per-Action Outcomes: HTTP ${o.http}, head "${o.created}" (expected "${n} created"), ${o.badges.length} rows, statuses {${badgeSet(o)}} (expected all "201 Created")`));
  await shot(page, S, `${nameShot}-outcomes`);
  await page.locator('#batch-done').click();
  await page.waitForTimeout(400);
  subs.push(sub(await page.locator('#batch-upload').isVisible(), 'Done returns to the Upload stage'));
  rec(id, subs);
}
await tryRow('6.2a', () => batchStep('6.2a', F_HOSP, bHosp, '6.2a-hospital'));
await tryRow('6.2b', () => batchStep('6.2b', F_PRAC, bPrac, '6.2b-practitioner'));

const HOSP_ID = 'https://github.com/synthetichealth/synthea|756ed90d-15f4-377d-b99f-ca1de5633481';
await tryRow('6.2c', async () => {
  const subs = [];
  const want = { Organization: 4, Location: 5, Practitioner: 4, PractitionerRole: 4 };
  const rail = await railCounts(Object.keys(want));
  await shot(page, S, '6.2c-rail-counts');
  const apiC = {}; for (const t of Object.keys(want)) apiC[t] = await count(t);
  subs.push(sub(Object.entries(want).every(([t, n]) => rail[t].n === n), `rail ${fmtRail(rail)} (expected Organization 4, Location 5, Practitioner 4, PractitionerRole 4; the ≈ form also passes)`));
  subs.push(sub(Object.entries(want).every(([t, n]) => apiC[t] === n), `API counts ${JSON.stringify(apiC)}`));
  const q = `/Organization?identifier=${HOSP_ID}`;
  const ui = await uiQuery(q, '6.2c-organization-identifier-search');
  const ap = await api(q, { headers: FH, save: 't2-6.2c-org.json' });
  const name = ap.json?.entry?.[0]?.resource?.name;
  subs.push(sub(ap.json?.entry?.length === 1 && name === 'MEDWAY COUNTRY MANOR SKILLED NURSING & REHABILITAT', `GET ${q} -> API ${ap.json?.entry?.length} result, name "${name}" (expected 1, MEDWAY COUNTRY MANOR SKILLED NURSING & REHABILITAT)`));
  subs.push(sub(ui.total === 1, `UI meta "${ui.meta}" (expected 1 result)`));
  rec('6.2c', subs);
});

// ================= 6.3 =================
let LPID = '';
await tryRow('6.3a', async () => {
  if (S3) return record(S, '6.3a', 'na', 'S3 backend: transactions refused (see 6.1)');
  const subs = [];
  await gotoBatch();
  const kind = await upload(F_PAT);
  if (kind !== 'preflight') return rec('6.3a', [sub(false, `upload error: ${await txt(page.locator('#batch-upload-error'))}`)]);
  const ex = await execute({});
  if (!ex.resp) return rec('6.3a', [sub(false, `transaction failed: ${await txt(page.locator('#batch-execute-error'))}`)]);
  const o = await outcome();
  subs.push(sub(o.http === '200' && o.created === '662 created' && allCreated(o, 662), `Per-Action Outcomes: HTTP ${o.http}, head "${o.created}" (expected "662 created"), ${o.badges.length} rows, statuses {${badgeSet(o)}} (expected all "201 Created"), summary "${o.summary}"`));
  await page.evaluate(() => window.scrollTo(0, 0));
  await shot(page, S, '6.3a-per-action-outcomes');
  await page.locator('#batch-done').click();
  await page.waitForTimeout(400);
  subs.push(sub(await page.locator('#batch-upload').isVisible(), 'Done returns to the Upload stage'));
  rec('6.3a', subs);
});

await tryRow('6.3b', async () => {
  if (S3) return record(S, '6.3b', 'na', 'S3 backend: transactions refused (see 6.1)');
  const subs = [];
  const q = '/Patient?given=Nicky270&family=Larkin917&birthdate=1996-04-19';
  const ui = await uiQuery(q, '6.3b-patient-search');
  const ap = await api(q, { headers: FH, save: 't2-6.3b-patient.json' });
  LPID = ap.json?.entry?.[0]?.resource?.id || '';
  subs.push(sub(ap.json?.entry?.length === 1 && ui.total === 1, `GET ${q} -> API ${ap.json?.entry?.length} result, UI "${ui.meta}" (expected 1); LPID=${LPID}`));
  if (LPID) {
    await page.locator('#query-results-body tr').first().locator('a, td').first().click();
    await page.locator('#resource-modal:not([hidden])').waitFor({ timeout: 10000 });
    await page.waitForTimeout(800);
    const modal = await txt(page.locator('#resource-modal'));
    subs.push(sub(modal.includes(`Patient/${LPID}`), `modal subject line contains Patient/${LPID}: ${modal.includes(`Patient/${LPID}`)}`));
    await shot(page, S, '6.3b-patient-modal');
    await page.keyboard.press('Escape'); await page.waitForTimeout(300);
    for (const [t, n] of [['Encounter', 49], ['Observation', 106], ['Condition', 33]]) {
      const qq = `/${t}?subject=Patient/${LPID}`;
      const u = await uiQuery(qq, `6.3b-${t.toLowerCase()}-count`);
      const a = await api(qq + '&_total=accurate', { headers: FH, save: `t2-6.3b-${t}.json` });
      subs.push(sub(a.json?.total === n && u.total === n, `GET ${qq} -> API ${a.json?.total}, UI "${u.meta}" (expected ${n})`));
    }
  }
  const tot = { Patient: await count('Patient') };
  subs.push(sub(tot.Patient === 1, `server totals ${JSON.stringify(tot)} (Patient expected 1 on the empty server)`));
  rec('6.3b', subs);
});

await tryRow('6.3c', async () => {
  if (S3) return record(S, '6.3c', 'na', 'S3 backend: transactions refused (see 6.1)');
  if (!LPID) return record(S, '6.3c', 'fail', 'LPID not found in 6.3b');
  const subs = [];
  const orgIds = new Set(), prIds = new Set();
  for (const t of ['Organization', 'Practitioner']) {
    const r = await api(`/${t}?_count=100`, { headers: FH });
    for (const e of r.json?.entry || []) (t === 'Organization' ? orgIds : prIds).add(e.resource.id);
  }
  const all = [];
  const r = await api(`/Encounter?subject=Patient/${LPID}&_count=100`, { headers: FH });
  for (const e of r.json?.entry || []) all.push(e.resource);
  const bad = all.filter((e) => {
    const sp = e.serviceProvider?.reference, pi = e.participant?.[0]?.individual?.reference, sj = e.subject?.reference;
    return !(/^Organization\/[^?]+$/.test(sp || '') && orgIds.has(sp.split('/')[1]) && /^Practitioner\/[^?]+$/.test(pi || '') && prIds.has(pi.split('/')[1]) && sj === `Patient/${LPID}`);
  });
  const e0 = all[0];
  subs.push(sub(all.length === 49 && !bad.length, `${all.length} Encounters read via API; ${bad.length} with non-literal/unknown serviceProvider, participant[0].individual or subject != Patient/${LPID}; sample: serviceProvider ${e0?.serviceProvider?.reference}, participant[0].individual ${e0?.participant?.[0]?.individual?.reference}, subject ${e0?.subject?.reference}; ${orgIds.size} Organizations, ${prIds.size} Practitioners in server`));
  await uiQuery(`/Encounter?subject=Patient/${LPID}`, null);
  await page.locator('#query-results-body tr').first().locator('a, td').first().click();
  await page.locator('#resource-modal:not([hidden])').waitFor({ timeout: 10000 });
  await page.waitForTimeout(1000);
  const mt = await page.locator('#resource-modal .cm-content, #resource-modal textarea').first().innerText().catch(() => '');
  const mSp = mt.match(/serviceProvider[\s\S]{0,80}?"reference":\s*"([^"]+)"/);
  subs.push(sub(mSp && /^Organization\/[^?]+$/.test(mSp[1]), `modal Encounter serviceProvider.reference "${mSp ? mSp[1] : 'not found in visible editor text'}"`));
  await shot(page, S, '6.3c-encounter-modal');
  await page.keyboard.press('Escape'); await page.waitForTimeout(300);
  const rail = await railCounts(['Organization']);
  subs.push(sub(rail.Organization.n === 4, `Organization rail ${fmtRail(rail)} (expected 4 or ≈4)`));
  rec('6.3c', subs);
});

// ================= 6.4 =================
async function refuse(id, file, re, expectText, name) {
  await gotoBatch();
  const kind = await upload(file);
  const err = await txt(page.locator('#batch-upload-error'));
  await shot(page, S, name);
  const stay = (await page.locator('#batch-upload').isVisible()) && !(await page.locator('#batch-preflight').isVisible());
  record(S, id, kind === 'error' && stay && re.test(err) ? 'pass' : 'fail', `${path.basename(file)} -> "${err}" (expected ${expectText}); page stays on Upload=${stay}`);
}
await tryRow('6.4a', () => refuse('6.4a', path.join(FX, 'not-a-bundle.json'), /^That JSON is not a FHIR Bundle\.$/, '"That JSON is not a FHIR Bundle."', '6.4a-not-a-bundle'));
await tryRow('6.4b', () => refuse('6.4b', path.join(FX, 'collection.json'), /^Only Bundles of type batch or transaction can be executed here\.$/, '"Only Bundles of type batch or transaction can be executed here."', '6.4b-collection'));
await tryRow('6.4c', () => refuse('6.4c', path.join(FX, 'not-json.log'), /^That file is not valid JSON\. \(.+\)$/, '"That file is not valid JSON. (<parser detail>)"', '6.4c-not-json'));

await browser.close();
