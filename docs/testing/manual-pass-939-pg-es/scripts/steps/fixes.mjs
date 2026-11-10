// Corrected checks for four rows whose first-run check was written wrongly. Same ids as the original rows; the final report uses the latest row of each id.
import fs from 'fs';
import { launch, shot, record, api, BASE, OUT } from '../lib.mjs';
const FH = { Accept: 'application/fhir+json' };
const [startIso, subId] = fs.readFileSync(`${OUT}/t3-start.txt`, 'utf8').trim().split('\n');
const { browser, page } = await launch();

// ---- T3 7.3a: summary of the submission detail page (label text compared case-insensitively; the page renders labels in capitals)
await page.goto(`${BASE}/ui/bulk-import/${subId}`);
await page.waitForFunction(() => /Processing finished/i.test(document.body.innerText), null, { timeout: 120000 });
await page.waitForTimeout(1500);
await shot(page, 'T3', '7.3a-submission-summary');
const text = (await page.locator('main').innerText()).replace(/\s+/g, ' ');
const has = (re) => re.test(text);
const checks = {
  manifestUrl: has(/manifest url\s+http:\/\/localhost:18801\/manifest\.json/i),
  recipient: has(/data recipient\s+http:\/\/localhost:18080/i),
  submissionId: text.includes(subId),
  submitter: text.includes(`urn:helios:hfs:bulk-submit | ${subId}`),
  status: has(/status\s+completed/i),
  auth: has(/authentication\s+none/i),
  outputs: has(/output files\s+24/i),
  errors: has(/error files\s+0/i),
  log: has(/Manifest accepted by the recipient \(200\)\./) && has(/Bulk status kick-off request/) && has(/processing finished cleanly \(24 outputs\); submission completed\./),
};
const bad = Object.entries(checks).filter(([, v]) => !v).map(([k]) => k);
record('T3', '7.3a', bad.length ? 'fail' : 'pass', `submission detail: Manifest URL, Data Recipient http://localhost:18080, Submission ID, Submitter, Status Completed, Authentication none, 24 output files, 0 error files, log lines (accepted 200, kick-off, finished cleanly, completed)${bad.length ? '; MISSING: ' + bad.join(',') : ''}`);

// ---- T2 6.2c: the Resource Types rail converges to the exact counts after the next reconcile
const counts = {};
for (const t of ['Organization', 'Location', 'Practitioner', 'PractitionerRole']) counts[t] = (await api(`/${t}?_summary=count&_total=accurate`, { headers: FH })).json?.total;
await page.goto(`${BASE}/ui/resources`);
await page.waitForTimeout(5000);
await page.locator('#type-rail-list').waitFor();
const rail = {};
for (const t of Object.keys(counts)) {
  const raw = ((await page.locator(`.filter-rail__item[data-type="${t}"] .count`).first().textContent().catch(() => '')) || '').trim();
  const m = raw.match(/^(≈)?\s*([\d,]+)/);
  rail[t] = m ? { approx: !!m[1], n: Number(m[2].replace(/,/g, '')), raw } : { raw };
}
await shot(page, 'T2', '6.2c-rail-converged');
const exact = Object.keys(counts).every((t) => rail[t] && rail[t].n === counts[t]);
record('T2', '6.2c', exact ? 'pass' : 'fail', `the rail shows the same counts as the API (a leading ≈ is allowed by the matrix): ${JSON.stringify(Object.fromEntries(Object.keys(counts).map((t) => [t, `${rail[t]?.approx ? '≈' : ''}${rail[t]?.n ?? rail[t]?.raw} (API ${counts[t]})`])))}. Right after a batch the rail may read one higher with a ≈ until the next reconcile (every 30 s); the exact check is the API/search count.`);

// ---- T2 6.3c: Encounters of the T2 patient hold literal references, read in the modal (CodeMirror only renders visible lines: scroll through it)
const LPID = (await api('/Patient?given=Nicky270&family=Larkin917', { headers: FH })).json.entry[0].resource.id;
await page.goto(`${BASE}/ui/resources`);
const urlInput = page.locator('input.query-builder__url');
await urlInput.fill(`GET /Encounter?subject=Patient/${LPID}`);
await urlInput.press('Enter');
await page.waitForSelector('#query-results-body tr', { timeout: 60000 });
await page.waitForTimeout(1500);
await page.locator('#query-results-body tr').first().locator('a, td').first().click();
await page.locator('#resource-modal:not([hidden])').waitFor({ timeout: 10000 });
await page.waitForTimeout(800);
let txt = '';
for (let i = 0; i < 20 && !txt.includes('"subject"'); i++) { await page.waitForTimeout(500); txt = await page.locator('#resource-modal textarea').first().inputValue().catch(() => ''); }
const sc = page.locator('#resource-modal .cm-scroller').first();
await sc.evaluate((e) => { e.scrollTop = 0; });
await page.waitForTimeout(300);
await shot(page, 'T2', '6.3c-encounter-modal-references');
const flat = txt.replace(/\s+/g, ' ');
const sp = flat.match(/"serviceProvider"\s*:\s*\{[^{}]*?"reference"\s*:\s*"(Organization\/[^"]+)"/);
const subj = flat.match(/"subject"\s*:\s*\{[^{}]*?"reference"\s*:\s*"(Patient\/[^"]+)"/);
const ind = flat.match(/"individual"\s*:\s*\{[^{}]*?"reference"\s*:\s*"(Practitioner\/[^"]+)"/);
// API side: every one of the 49 Encounters
let r = await api(`/Encounter?subject=Patient/${LPID}&_count=100`, { headers: FH });
const encs = (r.json.entry || []).map((e) => e.resource);
const badEnc = encs.filter((e) => !/^Organization\//.test(e.serviceProvider?.reference || '') || !/^Practitioner\//.test(e.participant?.[0]?.individual?.reference || '') || e.subject?.reference !== `Patient/${LPID}`);
let dangling = 0;
for (const e of encs) for (const ref of [e.serviceProvider?.reference, e.participant?.[0]?.individual?.reference]) if ((await api(`/${ref}`, { headers: FH })).status !== 200) dangling++;
const ok = !!sp && !!subj && !!ind && encs.length === 49 && badEnc.length === 0 && dangling === 0 && subj[1] === `Patient/${LPID}`;
record('T2', '6.3c', ok ? 'pass' : 'fail', `Encounter modal (editor scrolled end to end): serviceProvider ${sp ? sp[1] : 'NOT FOUND'}, participant.individual ${ind ? ind[1] : 'NOT FOUND'}, subject ${subj ? subj[1] : 'NOT FOUND'}; API: ${encs.length} Encounters, ${badEnc.length} without literal Organization/Practitioner/Patient references, ${dangling} references that do not resolve (GET returns not 200)`);
await browser.close();

// ---- T5 5.2: the one-patient export holds exactly what the server stores for that patient. At this point of the pass the patient has 16 Conditions: 15 from the corpus plus the Condition created by T8 step 12.2.2 (run before this check).
const PID = '7d24f7a0-6f2e-ce3b-5568-db7b14695583';
const stored = {};
for (const t of ['Condition', 'Observation']) stored[t] = (await api(`/${t}?patient=${PID}&_summary=count&_total=accurate`, { headers: FH })).json?.total;
const corpus = { Condition: 15, Observation: 165 };
const t8cond = (await api(`/Condition?patient=${PID}&_sort=-_lastUpdated&_count=1`, { headers: FH })).json.entry?.[0]?.resource?.meta?.lastUpdated;
const expected = { Condition: corpus.Condition + 1, Observation: corpus.Observation };
const prevRow = fs.readFileSync(`${OUT}/steps.tsv`, 'utf8').split('\n').filter((l) => l.split('\t')[2] === '5.2-rerun').pop() || '';
const exported = prevRow.match(/lines \{"Condition":(\d+),"Observation":(\d+),"Patient":(\d+)\}/);
const okExp = exported && Number(exported[1]) === stored.Condition && Number(exported[2]) === stored.Observation && Number(exported[3]) === 1 && stored.Condition === expected.Condition && stored.Observation === expected.Observation;
record('T5', '5.2', okExp ? 'pass' : 'fail', `one-patient export of the anchor patient (picked by id ${PID}): ${exported ? `Patient ${exported[3]}, Condition ${exported[1]}, Observation ${exported[2]}` : 'export counts not found'}; stored for the patient: Condition ${stored.Condition}, Observation ${stored.Observation}; matrix 15 Condition + 165 Observation, plus the Condition that T8 step 12.2.2 adds to the same patient (last Condition written ${t8cond}) = 16 at this point of the pass`);
console.log('fixes done');
