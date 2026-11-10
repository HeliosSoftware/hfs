// T3 7.3: poll the submission detail page, log progress, and capture the completed state.
import fs from 'fs';
import { launch, shot, record, OUT, BASE, api } from '../lib.mjs';
const [startIso, id] = fs.readFileSync(`${OUT}/t3-start.txt`, 'utf8').trim().split('\n');
const url = `${BASE}/ui/bulk-import/${id}`;
const log = `${OUT}/t3-progress.tsv`;
let last = '';
for (;;) {
  let t = '';
  try { t = (await (await fetch(url)).text()).replace(/<script[\s\S]*?<\/script>/g, '').replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' '); } catch (e) { t = 'fetch error ' + e.message; }
  const status = (t.match(/STATUS\s+(In Progress|Completed|Failed|[A-Za-z ]{3,20}?)\s+AUTH/i) || [])[1] || '?';
  const m = t.match(/(Processing finished[^.]*\.|Waiting for the recipient[^.]*\.|PROCESSING[^|]{0,200})/i);
  const snap = `${status}\t${(m ? m[1] : t.slice(t.indexOf('STATUS'), t.indexOf('STATUS') + 200)).slice(0, 220)}`;
  fs.appendFileSync(log, `${new Date().toISOString()}\t${snap}\n`);
  if (/^(Completed|Failed)/i.test(status)) { last = status; break; }
  await new Promise((r) => setTimeout(r, 60000));
}
const end = new Date().toISOString();
const { browser, page } = await launch();
await page.goto(url);
await page.waitForTimeout(3000);
await shot(page, 'T3', '7.3-submission-detail-final');
await shot(page, 'T3', '7.3-submission-detail-final-full', { full: true });
const text = await page.locator('main').innerText();
const secs = Math.round((Date.parse(end) - Date.parse(startIso)) / 1000);
fs.writeFileSync(`${OUT}/t3-end.txt`, `${end}\n${secs}\n`);
const outputs = (text.match(/Output files\s*\n?\s*(\d+)/i) || [])[1];
const errs = (text.match(/Error files\s*\n?\s*(\d+)/i) || [])[1];
record('T3', '7.3b', last === 'Completed' && outputs === '24' && errs === '0' ? 'pass' : 'fail', `status ${last}; ingest wall-clock ${secs}s (${startIso} -> ${end}); output files ${outputs}; error files ${errs}; text: ${text.replace(/\s+/g, ' ').slice(0, 500)}`);
await browser.close();
