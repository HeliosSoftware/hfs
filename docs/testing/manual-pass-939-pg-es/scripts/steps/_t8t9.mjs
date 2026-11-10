// Shared helpers for steps/t8.mjs and steps/t9.mjs (owner: t8t9). Does not modify lib.mjs.
import fs from 'fs';
import net from 'net';
import { spawn, execSync } from 'child_process';
import { BASE, api, shot, record } from '../lib.mjs';

export const HOME = process.env.HOME;
export const WORK = process.env.WORK || `${HOME}/tmp/939/work`;
export const FIXTURES = process.env.FIXTURES || `${WORK}/fixtures`;
export const WEBHOOK_PORT = Number(process.env.WEBHOOK_PORT || 9999);
export const WEBHOOK_LOG = process.env.WEBHOOK_LOG || `${WORK}/webhook.log`;
export const WEBHOOK_PID = process.env.WEBHOOK_PID || `${WORK}/webhook.pid`;
export const HFS_LOG = process.env.HFS_LOG || '';
export const PATIENT = process.env.T8_PATIENT || '7d24f7a0-6f2e-ce3b-5568-db7b14695583';
export const ENDPOINT = `http://127.0.0.1:${WEBHOOK_PORT}/webhook`;
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const RECEIVER = `
import http.server, json, sys
class H(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
        try: b = json.loads(body) if body else None
        except Exception: b = body.decode('utf-8','replace')
        print(json.dumps({"path": self.path, "auth": self.headers.get("Authorization"), "body": b}), flush=True)
        self.send_response(200); self.end_headers()
    def log_message(self, *a): pass
http.server.HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
`;

export function portOpen(port) {
  return new Promise((res) => {
    const s = net.connect(port, '127.0.0.1');
    s.on('connect', () => { s.destroy(); res(true); });
    s.on('error', () => res(false));
  });
}
export function receiverPid() { try { return Number(fs.readFileSync(WEBHOOK_PID, 'utf8')); } catch { return 0; } }
export async function startReceiver({ truncate = false } = {}) {
  fs.mkdirSync(WORK, { recursive: true });
  if (await portOpen(WEBHOOK_PORT)) return { started: false, pid: receiverPid() };
  if (truncate) fs.writeFileSync(WEBHOOK_LOG, '');
  const fd = fs.openSync(WEBHOOK_LOG, 'a');
  const p = spawn('python3', ['-u', '-c', RECEIVER, String(WEBHOOK_PORT)], { detached: true, stdio: ['ignore', fd, fd] });
  p.unref();
  fs.closeSync(fd);
  fs.writeFileSync(WEBHOOK_PID, String(p.pid));
  for (let i = 0; i < 50 && !(await portOpen(WEBHOOK_PORT)); i++) await sleep(100);
  return { started: true, pid: p.pid };
}
export async function stopReceiver() {
  const pid = receiverPid();
  if (pid) { try { process.kill(pid, 'SIGTERM'); } catch {} }
  for (let i = 0; i < 50 && (await portOpen(WEBHOOK_PORT)); i++) await sleep(100);
  return !(await portOpen(WEBHOOK_PORT));
}

export function logLines() {
  let t = ''; try { t = fs.readFileSync(WEBHOOK_LOG, 'utf8'); } catch {}
  return t.split('\n').filter((l) => l.trim()).map((l) => { try { return JSON.parse(l); } catch { return { raw: l }; } });
}
const flat = (o) => JSON.stringify(o);
export const isHandshake = (e) => /"handshake"/.test(flat(e.body));
export const eventLines = () => logLines().filter((e) => !isHandshake(e));
// recursively look for the events-since-subscription-start counter
export function eventsSince(o) {
  let found;
  (function walk(x) {
    if (found !== undefined || x === null || typeof x !== 'object') return;
    if (typeof x.name === 'string' && /events-?since-?subscription-?start/i.test(x.name)) {
      found = x.valueInteger64 ?? x.valueInteger ?? x.valueString ?? x.valueUnsignedInt;
      return;
    }
    for (const v of Object.values(x)) walk(v);
  })(o);
  return found === undefined ? undefined : Number(found);
}
export async function waitLines(n, timeoutMs = 20000, cmp = (a, b) => a >= b) {
  const t0 = Date.now();
  while (Date.now() - t0 < timeoutMs) { if (cmp(logLines().length, n)) break; await sleep(500); }
  return logLines().length;
}

export function sh(cmd, timeout = 180000) {
  try { return { ok: true, out: execSync(cmd, { timeout, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], shell: '/bin/bash' }) }; }
  catch (e) { return { ok: false, out: String(e.stdout || '') + String(e.stderr || e.message) }; }
}
export async function waitServer(timeoutMs = 120000) {
  const t0 = Date.now();
  while (Date.now() - t0 < timeoutMs) {
    try { const r = await fetch(BASE + '/metadata'); if (r.status === 200) return true; } catch {}
    await sleep(1000);
  }
  return false;
}
export function hfsLogCount(re) {
  if (!HFS_LOG || !fs.existsSync(HFS_LOG)) return null;
  const t = fs.readFileSync(HFS_LOG, 'utf8');
  return (t.match(re) || []).length;
}

export const TOPIC = {
  resourceType: 'Basic', id: 'manual-topic',
  code: { coding: [{ system: 'http://hl7.org/fhir/fhir-types', code: 'SubscriptionTopic' }] },
  extension: [
    { url: 'http://hl7.org/fhir/5.0/StructureDefinition/extension-SubscriptionTopic.url', valueUri: 'http://example.org/topics/encounter-start' },
    { url: 'http://hl7.org/fhir/5.0/StructureDefinition/extension-SubscriptionTopic.title', valueString: 'Encounter created' },
    { url: 'http://hl7.org/fhir/4.3/StructureDefinition/extension-SubscriptionTopic.resourceTrigger', extension: [
      { url: 'resource', valueUri: 'http://hl7.org/fhir/StructureDefinition/Encounter' },
      { url: 'supportedInteraction', valueCode: 'create' }] }],
};
export const SUB = {
  resourceType: 'Subscription', id: 'manual-sub', status: 'requested', reason: 'manual matrix',
  meta: { profile: ['http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-subscription'] },
  criteria: 'http://example.org/topics/encounter-start',
  channel: { type: 'rest-hook', endpoint: ENDPOINT, payload: 'application/fhir+json',
    header: ['Authorization: Bearer manual-token'],
    _payload: { extension: [{ url: 'http://hl7.org/fhir/uv/subscriptions-backport/StructureDefinition/backport-payload-content', valueCode: 'id-only' }] } },
};
const enc = () => ({ request: { method: 'POST', url: 'Encounter' }, resource: { resourceType: 'Encounter', status: 'in-progress',
  class: { system: 'http://terminology.hl7.org/CodeSystem/v3-ActCode', code: 'AMB' }, subject: { reference: `Patient/${PATIENT}` } } });
export const ENCOUNTERS = { resourceType: 'Bundle', type: 'batch', entry: [enc(), enc(), enc()] };
export function ensureEncountersFixture() {
  const f = `${FIXTURES}/encounters.json`;
  fs.mkdirSync(FIXTURES, { recursive: true });
  if (!fs.existsSync(f)) fs.writeFileSync(f, JSON.stringify(ENCOUNTERS, null, 1));
  return f;
}

// Replace the JSON pane of /ui/editor?type=X with `doc` and press Save Changes.
export async function editorSave(page, type, doc, { step, shotName } = {}) {
  page.on('dialog', (d) => d.accept().catch(() => {}));
  await page.goto(`${BASE}/ui/editor?type=${type}`, { waitUntil: 'load' });
  const cm = page.locator('#editor-body .cm-content').first();
  await cm.waitFor({ timeout: 20000 });
  await cm.click();
  await page.keyboard.press('ControlOrMeta+a');
  await page.keyboard.insertText(JSON.stringify(doc, null, 2));
  await page.waitForTimeout(800);
  const pillBefore = await page.locator('.tag--unsaved:visible').count();
  if (step && shotName) await shot(page, step, shotName + '-before-save');
  await page.click('#editor-save');
  let cleared = false;
  for (let i = 0; i < 40; i++) {
    await page.waitForTimeout(250);
    if ((await page.locator('.tag--unsaved:visible').count()) === 0) { cleared = true; break; }
  }
  await page.waitForTimeout(500);
  const status = (await page.locator('#editor-status').innerText().catch(() => '')).trim();
  if (step && shotName) await shot(page, step, shotName + '-saved');
  return { pillBefore, cleared, status };
}

// Upload encounters.json on the Batch page and Execute. Returns {request, created, overall, summary}.
export async function uploadBatch(page, step, tag) {
  const f = ensureEncountersFixture();
  await page.goto(`${BASE}/ui/batch`, { waitUntil: 'load' });
  await page.waitForSelector('#batch-file', { state: 'attached', timeout: 15000 });
  await page.setInputFiles('#batch-file', f);
  await page.waitForSelector('#batch-preflight:not([hidden])', { timeout: 15000 });
  const request = (await page.locator('#batch-request-line').innerText()).trim();
  const semantics = (await page.locator('#batch-semantics').innerText()).trim();
  if (step) await shot(page, step, `${tag}-preflight`);
  await page.click('#batch-execute-top');
  await page.waitForSelector('#batch-response:not([hidden])', { timeout: 60000 });
  await page.waitForTimeout(300);
  const created = (await page.locator('#batch-created').innerText()).trim();
  const overall = (await page.locator('#batch-overall').innerText()).trim();
  const summary = (await page.locator('#batch-summary').innerText()).trim();
  if (step) await shot(page, step, `${tag}-response`);
  return { request, semantics, created, overall, summary };
}

// Scrape /ui/subscriptions (reloads the page).
export async function scrapeSubs(page, sort) {
  await page.goto(`${BASE}/ui/subscriptions${sort ? `?sort=${sort}` : ''}`, { waitUntil: 'load' });
  return page.evaluate(() => {
    const t = (e) => (e ? e.textContent.replace(/\s+/g, ' ').trim() : '');
    const unavailable = !document.querySelector('.stat-grid');
    const cards = [...document.querySelectorAll('.stat-grid .stat')].map((s) => ({
      label: t(s.querySelector('.stat__label')), value: t(s.querySelector('.stat__value')), sub: t(s.querySelector('.stat__sub')) }));
    const rows = [...document.querySelectorAll('tbody tr:not(.data-table__empty)')].map((tr) => {
      const td = tr.querySelectorAll('td');
      const urls0 = td[0]?.querySelector('.url'); const urls1 = td[1]?.querySelector('.url');
      const poly = td[3]?.querySelector('polyline');
      return { id: t(td[0]?.querySelector('.code')), topic: t(urls0), topicTitle: urls0?.getAttribute('title') || '',
        channel: t(td[1]).replace(t(urls1), '').trim(), endpoint: t(urls1), endpointTitle: urls1?.getAttribute('title') || '',
        status: t(td[2]), last24: t(td[3]), sparkTitle: td[3]?.querySelector('svg title')?.textContent || '',
        sparkPoints: poly ? poly.getAttribute('points').trim().split(/\s+/).filter(Boolean).length : 0,
        sent: t(td[4]), streak: t(td[5]), alert: tr.classList.contains('row--alert') };
    });
    return { unavailable, notice: unavailable ? t(document.querySelector('.card.notice')) : '', cards, rows,
      sidebar: !!document.querySelector('a[href="/ui/subscriptions"]'), heading: t(document.querySelector('.toolbar__count')) };
  });
}
export const num = (s) => Number(String(s).replace(/[^\d]/g, ''));
export { BASE, api, shot, record };
