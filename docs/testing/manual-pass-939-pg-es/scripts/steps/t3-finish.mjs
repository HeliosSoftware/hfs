// T3 7.3/7.4/7.5(1,2): final detail page, rebuild confirmation from the log, and per-type counts vs corpus-counts.
import fs from 'fs';
import { launch, shot, record, OUT, BASE, api } from '../lib.mjs';
const [startIso, id] = fs.readFileSync(`${OUT}/t3-start.txt`, 'utf8').trim().split('\n');
const LOG = process.env.HFS_LOG;
const log = fs.readFileSync(LOG, 'utf8');
const { browser, page } = await launch();
await page.goto(`${BASE}/ui/bulk-import/${id}`);
await page.waitForFunction(() => /Processing finished/.test(document.body.innerText), null, { timeout: 120000 });
await page.waitForTimeout(1500);
await shot(page, 'T3', '7.3-submission-detail-final');
await shot(page, 'T3', '7.3-submission-detail-final-full', { full: true });
const text = (await page.locator('main').innerText()).replace(/\s+/g, ' ');
const fin = (text.match(/Processing finished at\s+(\S+)/) || [])[1];
const outs = (text.match(/Output files\s+(\d+)/i) || [])[1];
const errs = (text.match(/Error files\s+(\d+)/i) || [])[1];
const warn = (text.match(/([\d,]+) resources flagged with warnings/) || [])[1];
const completed = /Completed/.test(text) && /submission completed\./.test(text);
const lastIngest = (log.match(/^(\S+)\s+WARN .*resources ingested but not indexed for search during ingest/m) || [])[1];
const rejected = [...log.matchAll(/search index rejected ingested resources.*rejected=(\d+)/g)].reduce((a, m) => a + +m[1], 0);
const rebuild = log.match(/^(\S+)\s+INFO helios_persistence::search::reindex: reindex job finished .*outcome=(\w+) types_done=(\d+) types=(\d+) processed=(\d+) total=(\d+).*failed=(\d+)/m);
const gen = log.match(/^(\S+)\s+INFO .*deferred reindex generation completed/m);
const startMs = Date.parse(startIso), s = (iso) => Math.round((Date.parse(iso) - startMs) / 1000);
const hms = (n) => `${Math.floor(n / 3600)}h ${Math.floor((n % 3600) / 60)}m ${n % 60}s`;
fs.writeFileSync(`${OUT}/t3-end.txt`, `${fin}\n${s(fin)}\n`);
record('T3', '7.3b', completed && outs === '24' && errs === '0' ? 'pass' : 'fail', `status Completed=${completed}; Output files ${outs}; Error files ${errs}; warnings: ${warn} resources flagged (re-index); UI "Processing finished at" ${fin} = ${hms(s(fin))} after start ${startIso}. The finished instant is the first status poll after completion, no browser was polling: the server log shows ingest done at ${lastIngest} (${hms(s(lastIngest))}).`);
record('T3', '7.4', rebuild && rebuild[2] === 'completed' && rebuild[7] === '0' && gen ? 'pass' : 'fail', `mode DEFER_INDEXING=false (index during ingest). Index rejected ${rejected} resources in 9 pages (reason: writing to the search index timed out after 30s) at the memory peak; the deferred reindex re-indexed them: outcome=${rebuild && rebuild[2]}, processed ${rebuild && rebuild[5]} of ${rebuild && rebuild[6]}, failed=${rebuild && rebuild[7]}, finished ${rebuild && rebuild[1]} (searchable after ${rebuild ? hms(s(rebuild[1])) : '?'}); "deferred reindex generation completed" ${gen ? gen[1] : 'MISSING'}`);
// 7.5 counts via API, per type, vs corpus-counts + T2
const rows = fs.readFileSync(process.env.CORPUS_COUNTS || `${process.env.HOME}/hfs-t3/corpus/corpus-counts.tsv`, 'utf8').trim().split('\n').map((l) => l.split('\t'));
const t2 = { Patient: 1, Organization: 4, Practitioner: 4, PractitionerRole: 4, Location: 5, Encounter: 49, Observation: 106, Condition: 33 };
const lines = []; let bad = 0;
for (const [type, n] of rows) {
  const r = await api(`/${type}?_summary=count`);
  const want = +n + (t2[type] || 0);
  const ok = r.json && r.json.total === want; if (!ok) bad++;
  lines.push(`${type}\t${want}\t${r.json ? r.json.total : r.status}\t${ok ? 'ok' : 'MISMATCH'}`);
}
fs.writeFileSync(`${OUT}/outputs/t3-pg-counts.tsv`, 'type\texpected(corpus+T2)\tHFS_count\tresult\n' + lines.join('\n') + '\n');
record('T3', '7.5-counts-api', bad === 0 ? 'pass' : 'fail', `${rows.length} types vs corpus-counts + T2: ${bad} mismatches. ${lines.filter((l) => l.endsWith('MISMATCH')).join(' ; ')}`);
await browser.close();
