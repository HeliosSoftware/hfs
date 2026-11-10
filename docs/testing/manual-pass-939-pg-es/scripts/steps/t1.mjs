// T1 - Start HFS smoke (matrix section 5). Rows: 5.1 health, 5.2 metadata, 5.3 operations, 5.4 startup log,
// 5.5 dashboard counts, 5.6 clinical rail at 0, 5.7 sidebar R4, 5.8 version switch (R5).
// Env: HFS_BASE, OUT; HFS_LOG (server log path; unset => 5.4 na); BACKEND (default pg-es; names the backend expected in the log);
// ES_PREFIX (default hfs; expected index prefix in the log for -es composites); MULTI_VERSION=1 (run the R5 check; default: na, R4-only build).
import fs from 'fs';
import { launch, shot, record, api, BASE } from '../lib.mjs';

const S = 'T1';
const BACKEND = process.env.BACKEND || 'pg-es';
const COMPOSITE = /-es$/.test(BACKEND);
const PREFIX = process.env.ES_PREFIX || 'hfs';
const FH = { Accept: 'application/fhir+json' };
const num = (s) => Number(String(s).replace(/[^\d]/g, ''));
const tryRow = async (id, fn) => { try { await fn(); } catch (e) { record(S, id, 'fail', `script error: ${String(e.message || e).split('\n')[0]}`); } };

const { browser, page } = await launch();
page.setDefaultTimeout(15000);

await tryRow('5.1', async () => {
  const r = await api('/health', { save: 't1-health.json' });
  record(S, '5.1', r.status === 200 ? 'pass' : 'fail', `GET /health -> ${r.status} ${r.text.slice(0, 120)} (expected 200)`);
});

let meta;
await tryRow('5.2', async () => {
  const r = await api('/metadata', { headers: FH, save: 't1-metadata.json' });
  meta = r.json;
  const nres = meta?.rest?.[0]?.resource?.length;
  record(S, '5.2', r.status === 200 && meta?.fhirVersion === '4.0.1' ? 'pass' : 'fail',
    `GET /metadata -> ${r.status}, fhirVersion ${meta?.fhirVersion} (expected 4.0.1), software ${meta?.software?.name}, ${nres} resource types`);
});

await tryRow('5.3', async () => {
  const ops = (meta?.rest?.[0]?.operation || []).map((o) => o.name).sort();
  const need = ['export', 'sql-run', 'sql-export', 'bulk-submit'];
  const missing = need.filter((n) => !ops.includes(n));
  record(S, '5.3', ops.length && !missing.length ? 'pass' : 'fail', `operations: ${ops.join(' ')} (expected at least ${need.join(', ')}; missing: ${missing.join(',') || 'none'})`);
});

await tryRow('5.4', async () => {
  const log = process.env.HFS_LOG;
  if (!log) return record(S, '5.4', 'na', 'HFS_LOG not set');
  if (!fs.existsSync(log)) return record(S, '5.4', 'fail', `HFS_LOG ${log} does not exist`);
  const txt = fs.readFileSync(log, 'utf8');
  const line = txt.split('\n').find((l) => /storage_backend=/.test(l)) || '';
  const m = line.match(/storage_backend=(\S+)/);
  const subs = [];
  subs.push(`startup line storage_backend=${m ? m[1] : 'not found'} (expected ${BACKEND})` + (m && m[1] === BACKEND ? '' : ' MISMATCH'));
  let ok = !!m && m[1] === BACKEND;
  if (COMPOSITE) {
    const px = txt.split('\n').filter((l) => /index.?prefix/i.test(l));
    const has = px.some((l) => new RegExp(`index_prefix="?${PREFIX}\\b`).test(l));
    subs.push(`ES index prefix "${PREFIX}" ${has ? 'named' : 'NOT named'} in log (${px.length} prefix lines)`);
    ok = ok && has;
  }
  const seeded = txt.match(/Seeded spec SearchParameters into storage[^\n]*created=(\d+)/);
  subs.push(`Seeded SearchParameters created=${seeded ? seeded[1] : 'not found'} (expected 1372 on an empty server)`);
  if (!seeded || seeded[1] !== '1372') ok = false;
  const mism = /advertises a different port/.test(txt);
  subs.push(`base-url/port mismatch warning: ${mism ? 'PRESENT' : 'absent'}`);
  if (mism) ok = false;
  record(S, '5.4', ok ? 'pass' : 'fail', subs.join(' | '));
});

await tryRow('5.5', async () => {
  await page.goto(`${BASE}/ui`);
  await page.locator('.stat-grid').waitFor();
  await page.waitForTimeout(2500);
  const stored = (await page.locator('.stat', { hasText: /Stored resources/i }).first().locator('.stat__value').textContent()).trim();
  const chart = (await page.locator('#dash-chart .stat__value').first().textContent().catch(() => '')).trim();
  await shot(page, S, '5.5-dashboard');
  const sp = await api('/SearchParameter?_summary=count', { headers: FH });
  const cd = await api('/CompartmentDefinition?_summary=count', { headers: FH });
  const tot = (sp.json?.total ?? -1) + (cd.json?.total ?? -1);
  const okUi = /1\.4k|1,377/.test(stored) && (!chart || num(chart) === 1377);
  record(S, '5.5', okUi && sp.json?.total === 1372 && cd.json?.total === 5 ? 'pass' : 'fail',
    `dashboard Stored resources "${stored}" (expected ~1.4k), chart total "${chart}" (expected 1,377); API SearchParameter ${sp.json?.total} (expected 1372) + CompartmentDefinition ${cd.json?.total} (expected 5) = ${tot}`);
});

await tryRow('5.6', async () => {
  await page.goto(`${BASE}/ui/resources`);
  await page.locator('#type-rail-list').waitFor();
  const counts = {};
  for (const t of ['Patient', 'Encounter', 'Observation', 'Condition', 'Organization', 'Practitioner', 'Location']) {
    counts[t] = ((await page.locator(`.filter-rail__item[data-type="${t}"] .count`).first().textContent().catch(() => 'missing')) || '').trim().replace(/\s+/g, ' ');
  }
  const sp = ((await page.locator('.filter-rail__item[data-type="SearchParameter"] .count').first().textContent().catch(() => '')) || '').trim();
  await shot(page, S, '5.6-clinical-rail-zero');
  const nonzero = Object.entries(counts).filter(([, v]) => !/^0$/.test(v));
  record(S, '5.6', !nonzero.length && num(sp) === 1372 ? 'pass' : 'fail',
    `clinical rail ${JSON.stringify(counts)} (expected all 0); SearchParameter rail "${sp}" (expected 1,372)`);
});

await tryRow('5.7', async () => {
  await page.goto(`${BASE}/ui`);
  const sel = ((await page.locator('.selector__label').filter({ hasText: /FHIR/ }).first().textContent()) || '').trim();
  await shot(page, S, '5.7-sidebar-r4');
  record(S, '5.7', /\bR4$/.test(sel) ? 'pass' : 'fail', `sidebar FHIR selector reads "${sel}" (expected FHIR R4)`);
});

await tryRow('5.8', async () => {
  if (process.env.MULTI_VERSION !== '1') {
    return record(S, '5.8', 'na', 'R4-only build: no R5 available, version switch not applicable');
  }
  const r = await api('/metadata?_format=json', { headers: { Accept: 'application/fhir+json; fhirVersion=5.0' }, save: 't1-metadata-r5.json' });
  const sel = ((await (async () => { await page.goto(`${BASE}/ui`); return page.locator('.selector__label').filter({ hasText: /FHIR/ }).first().textContent(); })()) || '').trim();
  record(S, '5.8', r.json?.fhirVersion === '5.0.0' && /R4/.test(sel) ? 'pass' : 'fail', `fhirVersion=5.0 -> ${r.json?.fhirVersion} (expected 5.0.0); sidebar "${sel}" must be back on R4`);
});

await browser.close();
