// T8 Subscriptions (matrix section 12). Rows: 12.1, 12.2.1, 12.2.2, 12.2.3, 12.2.4, 12.3.1, 12.3.1-status, 12.3.1-events, 12.3.2, 12.3.3
// Env: HFS_BASE, OUT, WORK, WEBHOOK_PORT (9999), WEBHOOK_LOG, T8_PATIENT, T8_RETRY_WAIT_MS (default 30000), T8_RETRY_DELIVERY_TIMEOUT_MS (default 240000)
import fs from 'fs';
import { launch, BASE, api, shot, record } from '../lib.mjs';
import * as H from './_t8t9.mjs';
const S = 'T8';
const { browser, page } = await launch();
const consoleErrors = [];
page.on('console', (m) => { if (m.type() === 'error') consoleErrors.push(m.text()); });
page.on('pageerror', (e) => consoleErrors.push('pageerror: ' + e.message));
const failedLoads = [];
page.on('response', (r) => { if (r.status() >= 400) failedLoads.push(`${r.status()} ${r.url()}`); });
const guard = async (id, fn) => { try { await fn(); } catch (e) { record(S, id, 'fail', 'exception: ' + String(e.message).split('\n')[0]); } };

try {
  // setup: clean previous run state (idempotent)
  await api('/Subscription/manual-sub', { method: 'DELETE' });
  await api('/Basic/manual-topic', { method: 'DELETE' });
  await H.sleep(1500);
  const fix = H.ensureEncountersFixture();

  // 12.1 receiver
  await guard('12.1', async () => {
    await H.stopReceiver();
    const r = await H.startReceiver({ truncate: true });
    const up = await H.portOpen(H.WEBHOOK_PORT);
    record(S, '12.1', up ? 'pass' : 'fail', `receiver listening on 127.0.0.1:${H.WEBHOOK_PORT} pid=${r.pid} log=${H.WEBHOOK_LOG} (expected: listening, empty log); fixture ${fix}`);
  });

  // 12.2.1 topic
  await guard('12.2.1', async () => {
    const r = await H.editorSave(page, 'Basic', H.TOPIC, { step: S, shotName: '12.2.1-topic' });
    const g = await api('/Basic/manual-topic', { save: 't8-12.2.1-topic.json' });
    const ok = r.cleared && g.status === 200 && g.json?.code?.coding?.[0]?.code === 'SubscriptionTopic';
    record(S, '12.2.1', ok ? 'pass' : 'fail', `Unsaved pill before save=${r.pillBefore} cleared=${r.cleared}; editor status="${r.status}"; GET Basic/manual-topic -> ${g.status} (expected 200, code SubscriptionTopic)`);
  });

  // 12.2.2 subscription + 12.2.3 handshake
  const t0 = Date.now();
  await guard('12.2.2', async () => {
    const r = await H.editorSave(page, 'Subscription', H.SUB, { step: S, shotName: '12.2.2-subscription' });
    const g = await api('/Subscription/manual-sub', { save: 't8-12.2.2-subscription.json' });
    const ok = r.cleared && g.status === 200 && g.json?.channel?.endpoint === H.ENDPOINT;
    record(S, '12.2.2', ok ? 'pass' : 'fail', `Unsaved pill cleared=${r.cleared}; editor status="${r.status}"; GET Subscription/manual-sub -> ${g.status}, endpoint=${g.json?.channel?.endpoint} (expected ${H.ENDPOINT})`);
  });
  await guard('12.2.3', async () => {
    const n = await H.waitLines(1, 15000);
    const ls = H.logLines();
    const hs = ls.filter(H.isHandshake);
    const ok = hs.length === 1 && ls.length === 1 && ls[0].auth === 'Bearer manual-token';
    fs.copyFileSync(H.WEBHOOK_LOG, `${process.env.OUT || process.env.HOME + '/tmp/939/out'}/outputs/t8-webhook-after-handshake.log`);
    record(S, '12.2.3', ok ? 'pass' : 'fail', `after ${Date.now() - t0} ms: ${n} log line(s), handshake lines=${hs.length}, auth="${ls[0]?.auth}" (expected 1 line, handshake, "Bearer manual-token")`);
  });

  // 12.2.4 persisted status + history
  await guard('12.2.4', async () => {
    let g;
    for (let i = 0; i < 20; i++) { g = await api('/Subscription?_id=manual-sub&_elements=status', { save: 't8-12.2.4-status.json' }); if (g.json?.entry?.[0]?.resource?.status === 'active') break; await H.sleep(500); }
    const st = g.json?.entry?.[0]?.resource?.status;
    const h = await api('/Subscription/manual-sub/_history', { save: 't8-12.2.4-history.json' });
    const versions = (h.json?.entry || []).length;
    const asc = (h.json?.entry || []).map((e) => e.resource?.status).reverse();
    // UI path of the matrix: Resources -> run the query -> status column -> click the id -> History tab -> diff v1 -> v2
    await page.goto(`${BASE}/ui/resources`, { waitUntil: 'load' });
    const urlInput = page.locator('input.query-builder__url');
    await urlInput.fill('GET /Subscription?_id=manual-sub&_elements=status');
    await urlInput.press('Enter');
    await page.locator('#query-results-body tr').first().waitFor({ timeout: 20000 });
    await page.waitForTimeout(1500);
    const rowTxt = (await page.locator('#query-results-body').innerText()).replace(/\s+/g, ' ');
    await shot(page, S, '12.2.4a-query-status-column');
    let uiVersions = -1, diffText = '', uiNote = '';
    try {
      await page.locator('#query-results-body tr').first().locator('a, td').first().click();
      await page.locator('#resource-modal:not([hidden])').waitFor({ timeout: 10000 });
      await page.locator('[data-modal-tab="history"]').click();
      await page.waitForTimeout(2500);
      uiVersions = await page.locator('#resource-history-versions > *').count();
      diffText = (await page.locator('#resource-history-diff').innerText().catch(() => '')).replace(/\s+/g, ' ');
      await shot(page, S, '12.2.4b-history-diff');
    } catch (e) { uiNote = 'UI history navigation error: ' + String(e.message).split('\n')[0]; }
    const diffOk = /status/i.test(diffText) && /requested/.test(diffText) && /active/.test(diffText);
    const ok = st === 'active' && /active/.test(rowTxt) && versions === 2 && asc[0] === 'requested' && asc[1] === 'active' && uiVersions === 2 && diffOk;
    record(S, '12.2.4', ok ? 'pass' : 'fail', `UI result row="${rowTxt.slice(0, 80)}" (expected status column active); API ?_elements=status -> "${st}"; _history entries=${versions} (expected 2), statuses v1->v2=${asc.join('->')}; UI History tab version rows=${uiVersions} (expected 2); diff mentions status requested -> active=${diffOk} text="${diffText.slice(0, 160)}" ${uiNote}`);
  });

  // 12.3.1 trigger
  await guard('12.3.1', async () => {
    const before = H.logLines().length;
    const r = await H.uploadBatch(page, S, '12.3.1-upload');
    const n = await H.waitLines(before + 3, 30000);
    await H.sleep(2000);
    const ls = H.logLines();
    const last = ls[ls.length - 1];
    const ev = H.eventsSince(last?.body);
    fs.copyFileSync(H.WEBHOOK_LOG, `${process.env.OUT || process.env.HOME + '/tmp/939/out'}/outputs/t8-webhook-after-batch.log`);
    const uiOk = /3 entries/.test(r.request + ' ' + r.summary) || /batch/i.test(r.request);
    const ok = /3\s*created/i.test(r.created + ' ' + r.summary) && ls.length === 4 && last?.auth === 'Bearer manual-token' && ev === 3;
    record(S, '12.3.1', ok ? 'pass' : 'fail', `request line="${r.request}"; created badge="${r.created}"; overall="${r.overall}"; log lines=${ls.length} (expected 4); last auth="${last?.auth}"; events-since-subscription-start on last line=${ev} (expected 3); id-only payload`);
    await page.goto(`${BASE}/ui/subscriptions`, { waitUntil: 'load' });
  });
  await guard('12.3.1-status', async () => {
    const r = await api('/Subscription/manual-sub/$status', { save: 't8-12.3.1-status.json' });
    const n = H.eventsSince(r.json);
    const txt = r.text;
    const ok = r.status === 200 && /active/.test(txt) && n === 3;
    record(S, '12.3.1-status', ok ? 'pass' : 'fail', `GET Subscription/manual-sub/$status -> ${r.status}, status active=${/"active"/.test(txt)}, eventsSinceSubscriptionStart=${n} (expected 3)`);
  });
  await guard('12.3.1-events', async () => {
    const r = await api('/Subscription/manual-sub/$events?eventsSinceNumber=1&eventsUntilNumber=3', { save: 't8-12.3.1-events.json' });
    const entries = r.json?.entry || [];
    const types = entries.map((e) => e.resource?.resourceType || e.request?.url || '?');
    const evType = r.json?.type;
    const ok = r.status === 200 && r.json?.resourceType === 'Bundle' && entries.length >= 1;
    record(S, '12.3.1-events', ok ? 'pass' : 'fail', `GET $events?eventsSinceNumber=1&eventsUntilNumber=3 -> ${r.status}, Bundle.type=${evType}, entries=${entries.length} [${types.join(',')}]; the matrix does not state an expected count, expected status 200 with a history Bundle whose first entry is the SubscriptionStatus`);
  });

  // 12.3.2 non-matching
  await guard('12.3.2', async () => {
    const cond = { resourceType: 'Condition', subject: { reference: `Patient/${H.PATIENT}` } };
    const before = H.logLines().length;
    const r = await H.editorSave(page, 'Condition', cond, { step: S, shotName: '12.3.2-condition' });
    await H.sleep(5000);
    const after = H.logLines().length;
    record(S, '12.3.2', r.cleared && after === before && after === 4 ? 'pass' : 'fail', `Condition saved (pill cleared=${r.cleared}, status="${r.status}"); log lines before=${before} after=${after} (expected 4 and unchanged)`);
  });

  // 12.3.3 failure path
  await guard('12.3.3', async () => {
    const waitMs = Number(process.env.T8_RETRY_WAIT_MS || 30000);
    const down = await H.stopReceiver();
    const before = H.logLines().length;
    const r = await H.uploadBatch(page, S, '12.3.3-upload-receiver-down');
    await H.sleep(waitMs);
    const during = H.logLines().length;
    const t1 = Date.now();
    const rs = await H.startReceiver();
    const want = before + 3;
    const timeout = Number(process.env.T8_RETRY_DELIVERY_TIMEOUT_MS || 240000);
    const n = await H.waitLines(want, timeout);
    const waited = Date.now() - t1;
    await H.sleep(2000);
    const ls = H.logLines();
    const added = ls.slice(before);
    const authOk = added.every((e) => e.auth === 'Bearer manual-token');
    const evs = added.map((e) => H.eventsSince(e.body));
    fs.copyFileSync(H.WEBHOOK_LOG, `${process.env.OUT || process.env.HOME + '/tmp/939/out'}/outputs/t8-webhook-after-retry.log`);
    const ok = down && /3\s*created/i.test(r.created + r.summary) && during === before && ls.length === before + 3 && authOk;
    record(S, '12.3.3', ok ? 'pass' : 'fail', `receiver stopped=${down}; batch created="${r.created}"; log lines before=${before}, after ${waitMs} ms with receiver down=${during} (expected unchanged), after receiver restart +${waited} ms=${ls.length} (expected ${want}); added events-since counters=[${evs.join(',')}] auth all Bearer manual-token=${authOk}`);
  });

  // The Resource Editor probes GET /<Type>/<id>?_elements=id before saving a new id: 404 (or 410 after a delete) is the browser's own
  // "Failed to load resource" console line for that probe, not a defect. Ignore exactly that many such lines.
  const probes = failedLoads.filter((u) => /\/[A-Za-z]+\/[^/?]+\?_elements=id$/.test(u) && /^(404|410) /.test(u)).length;
  let toDrop = probes;
  const ce = consoleErrors.filter((e) => !/favicon/i.test(e)).filter((e) => !(toDrop > 0 && /Failed to load resource: the server responded with a status of (404|410)/.test(e) && toDrop--));
  record(S, '12.console', ce.length === 0 ? 'pass' : 'fail', ce.length ? ce.join(' | ').slice(0, 400) + ' ; failed responses: ' + failedLoads.join(', ') : `no browser console errors during T8 (ignoring ${probes} editor id-probe 404/410 lines)`);
} finally {
  await browser.close();
}
process.exit(0); // detached receiver / sockets can otherwise keep node alive
