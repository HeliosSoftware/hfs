// T9 Subscription activity dashboard (matrix section 13). Run after t8 (needs manual-sub active + receiver).
// Rows: 13.1 cards, 13.2 row, 13.3 repeat upload, 13.4a..13.4f failure path (full) or 13.4a/13.4s (short), 13.5 restart, 13.6 disabled, 13.7 console, 13.8 no-JS, 13.9 main dashboard
// Env: HFS_BASE OUT WORK WEBHOOK_PORT WEBHOOK_LOG HFS_LOG (server log file; optional)
//   T9_FAILURE_MODE=full (default; ~20 min: waits for the Error chip, then reactivates) | short (30 s + recovery before the 3rd window ends)
//   HFS_RESTART_CMD            shell command: restart HFS normally (engine enabled, rehydrate on); must return once the process is launched (the script then polls /metadata)
//   HFS_STOP_CMD + HFS_START_CMD  used instead when HFS_RESTART_CMD is unset (stop; then start)
//   HFS_RESTART_DISABLED_CMD   shell command: restart HFS with HFS_SUBSCRIPTIONS_ENABLED=false. When unset, HFS_STOP_CMD then HFS_START_CMD run with HFS_SUBSCRIPTIONS_ENABLED=false in the environment
//                              (the start script must NOT hard-code HFS_SUBSCRIPTIONS_ENABLED=true; use ${HFS_SUBSCRIPTIONS_ENABLED:-true})
//   T9_ERROR_TIMEOUT_MS (default 1080000 = 18 min: matrix 15-17 min + 1 min slack)
import { launch, BASE, api, shot, record } from '../lib.mjs';
import * as H from './_t8t9.mjs';
const S = 'T9';
const MODE = process.env.T9_FAILURE_MODE || 'full';
const { browser, page } = await launch();
const consoleErrors = [];
page.on('console', (m) => { if (m.type() === 'error') consoleErrors.push(m.text()); });
page.on('pageerror', (e) => consoleErrors.push('pageerror: ' + e.message));
const guard = async (id, fn) => { try { await fn(); } catch (e) { record(S, id, 'fail', 'exception: ' + String(e.message).split('\n')[0]); } };
const row = (d) => d.rows.find((r) => r.id === 'manual-sub');
const fmt = (r) => r ? `status="${r.status}" sent=${r.sent} streak=${r.streak} last24="${r.last24}" spark=${r.sparkTitle}/${r.sparkPoints}pts alert=${r.alert}` : 'row missing';
const eventCount = () => H.eventLines().length;
// returns a shell command string, or '' when no hook is available
const normalCmd = () => process.env.HFS_RESTART_CMD || (process.env.HFS_STOP_CMD && process.env.HFS_START_CMD ? `${process.env.HFS_STOP_CMD}; ${process.env.HFS_START_CMD}` : '');
const disabledCmd = () => process.env.HFS_RESTART_DISABLED_CMD || (process.env.HFS_STOP_CMD && process.env.HFS_START_CMD ? `${process.env.HFS_STOP_CMD}; HFS_SUBSCRIPTIONS_ENABLED=false ${process.env.HFS_START_CMD}` : '');

try {
  // setup: receiver + subscription must exist (T8 normally did this)
  await H.startReceiver();
  let g = await api('/Subscription/manual-sub');
  if (g.status !== 200) {
    await api('/Basic/manual-topic', { method: 'PUT', headers: { 'content-type': 'application/fhir+json' }, body: JSON.stringify(H.TOPIC) });
    await api('/Subscription/manual-sub', { method: 'PUT', headers: { 'content-type': 'application/fhir+json' }, body: JSON.stringify(H.SUB) });
    await H.sleep(4000);
    console.log('setup: manual-sub was missing; created via API (T8 not run first)');
  }

  // 13.1
  let d1;
  await guard('13.1', async () => {
    d1 = await H.scrapeSubs(page);
    await shot(page, S, '13.1-status-cards');
    const [a, f, i, dl] = d1.cards;
    const expDelivered = eventCount();
    const ok = a?.value === '1' && /delivering/.test(a.sub) && f?.value === '0' && /Needs attention/.test(f.sub) && i?.value === '0' && /No clients/.test(i.sub)
      && H.num(dl?.value) === expDelivered && /\d+% first try/.test(dl.sub);
    record(S, '13.1', ok ? 'pass' : 'fail', `cards: ${d1.cards.map((c) => `${c.label}=${c.value} (${c.sub})`).join('; ')}; expected Active 1 (delivering), Failing 0 (Needs attention), Idle 0 (No clients), Delivered = ${expDelivered} event lines in webhook log (handshakes excluded) with "N% first try"`);
  });

  // 13.2
  await guard('13.2', async () => {
    const r = row(d1 || await H.scrapeSubs(page));
    await shot(page, S, '13.2-table-row');
    const sentExp = eventCount();
    const ok = r && r.topic === 'encounter-start' && r.topicTitle === 'http://example.org/topics/encounter-start' && r.channel === 'rest-hook'
      && r.endpoint === H.ENDPOINT && r.endpointTitle === H.ENDPOINT && /^Active$/.test(r.status) && r.sparkTitle !== '' && H.num(r.sent) === sentExp && r.streak === '0';
    record(S, '13.2', ok ? 'pass' : 'fail', `row: id=${r?.id} topic="${r?.topic}" (title attr "${r?.topicTitle}") channel="${r?.channel}" endpoint="${r?.endpoint}" ${fmt(r)}; expected topic encounter-start, rest-hook, ${H.ENDPOINT}, Active, sparkline tooltip with the 24h count, Sent=${sentExp}, streak 0`);
  });

  // 13.3
  await guard('13.3', async () => {
    const b = await H.scrapeSubs(page);
    const rb = row(b);
    const up = await H.uploadBatch(page, S, '13.3-upload');
    await H.waitLines(H.logLines().length + 3, 20000, () => eventCount() >= H.num(rb.sent) + 3);
    await H.sleep(2500);
    const a = await H.scrapeSubs(page);
    await shot(page, S, '13.3-after-upload');
    const ra = row(a);
    const dB = H.num(b.cards[3].value), dA = H.num(a.cards[3].value);
    const ok = /3\s*created/i.test(up.created + up.summary) && dA === dB + 3 && H.num(ra.sent) === H.num(rb.sent) + 3 && ra.sparkPoints >= rb.sparkPoints && ra.sparkTitle !== rb.sparkTitle;
    record(S, '13.3', ok ? 'pass' : 'fail', `Delivered ${dB}->${dA} (expected +3); Sent ${rb.sent}->${ra.sent} (expected +3); sparkline points ${rb.sparkPoints}->${ra.sparkPoints}, tooltip "${rb.sparkTitle}"->"${ra.sparkTitle}" (a new point appears only when the current half-hour bucket is a new one; same bucket just raises its value)`);
  });

  // 13.4 failure path
  const streaks = {};
  if (MODE === 'short') {
    await guard('13.4a', async () => {
      const before = await H.scrapeSubs(page);
      const sentBefore = H.num(row(before).sent);
      const down = await H.stopReceiver();
      const nBefore = H.logLines().length;
      const up = await H.uploadBatch(page, S, '13.4-upload-receiver-down');
      await H.sleep(30000);
      const d = await H.scrapeSubs(page);
      await shot(page, S, '13.4a-30s-receiver-down');
      const r = row(d);
      const cf = H.hfsLogCount(/Connection failed/g);
      const ok = down && /^Active$/.test(r.status) && d.cards[1].value === '0' && r.streak === '0' && (cf === null || cf > 0);
      record(S, '13.4a', ok ? 'pass' : 'fail', `30 s after upload with receiver down: ${fmt(r)}; Failing card=${d.cards[1].value}; "Connection failed" lines in HFS log=${cf === null ? 'n/a (HFS_LOG not set)' : cf}; expected Active, Failing 0, streak 0`);
      globalThis.__sentBefore = sentBefore; globalThis.__nBefore = nBefore;
    });
    await guard('13.4s', async () => {
      await H.startReceiver();
      const want = globalThis.__nBefore + 3;
      const n = await H.waitLines(want, 240000);
      await H.sleep(3000);
      const d = await H.scrapeSubs(page);
      await shot(page, S, '13.4s-recovered-before-3rd-window');
      const r = row(d);
      const ok = n >= want && /^Active$/.test(r.status) && r.streak === '0' && d.cards[1].value === '0';
      record(S, '13.4s', ok ? 'pass' : 'fail', `receiver restarted before the 3rd retry window ended: log lines ${globalThis.__nBefore}->${n} (expected +3); ${fmt(r)}; expected Active, streak 0 (short mode does not wait the 15-17 minutes for the Error chip)`);
    });
    for (const id of ['13.4b', '13.4c', '13.4d', '13.4e', '13.4f']) record(S, id, 'na', 'T9_FAILURE_MODE=short: the 5/10/15-17 minute streak, Error chip, Sort and reactivation checks are only run in full mode');
  } else {
    let nBefore;
    await guard('13.4a', async () => {
      const down = await H.stopReceiver();
      nBefore = H.logLines().length;
      globalThis.__t0 = Date.now();
      const up = await H.uploadBatch(page, S, '13.4-upload-receiver-down');
      globalThis.__t0 = Date.now();
      await H.sleep(30000);
      const d = await H.scrapeSubs(page);
      await shot(page, S, '13.4a-30s-receiver-down');
      const r = row(d);
      const cf = H.hfsLogCount(/Connection failed/g);
      const ok = down && /3\s*created/i.test(up.created + up.summary) && /^Active$/.test(r.status) && d.cards[1].value === '0' && r.streak === '0' && (cf === null || cf > 0);
      record(S, '13.4a', ok ? 'pass' : 'fail', `30 s after upload, receiver down: ${fmt(r)}; Failing card=${d.cards[1].value}; "Connection failed" lines in HFS log=${cf === null ? 'n/a (HFS_LOG not set)' : cf}; expected Active, Failing 0, streak 0`);
    });
    // poll until Error or timeout
    const tmo = Number(process.env.T9_ERROR_TIMEOUT_MS || 1080000);
    let errAt = null, last;
    while (Date.now() - globalThis.__t0 < tmo) {
      await H.sleep(15000);
      const d = await H.scrapeSubs(page);
      const r = row(d); last = { d, r };
      const el = Math.round((Date.now() - globalThis.__t0) / 1000);
      const s = r?.streak;
      if (s && !(s in streaks)) { streaks[s] = el; if (s === '1' || s === '2') await shot(page, S, `13.4-streak-${s}`); }
      if (r && /^Error$/.test(r.status)) { errAt = el; break; }
    }
    const mm = (s) => (s === undefined ? 'never seen' : `${Math.floor(s / 60)} min ${s % 60} s after upload`);
    record(S, '13.4b', streaks['1'] !== undefined && streaks['1'] >= 240 && streaks['1'] <= 420 ? 'pass' : 'fail', `streak 1 first seen ${mm(streaks['1'])} (expected about 5 min, each right after a "Max retries exhausted" line; 15 s polling)`);
    record(S, '13.4c', streaks['2'] !== undefined && streaks['2'] >= 540 && streaks['2'] <= 720 ? 'pass' : 'fail', `streak 2 first seen ${mm(streaks['2'])} (expected about 10 min)`);
    await guard('13.4d', async () => {
      const d = last?.d, r = last?.r;
      await shot(page, S, '13.4d-error-chip');
      const log = H.hfsLogCount(/consecutive_failures=3/g);
      const ok = errAt !== null && errAt >= 780 && errAt <= 1100 && d.cards[1].value === '1' && r.streak === '3' && r.alert;
      record(S, '13.4d', ok ? 'pass' : 'fail', `Error chip ${errAt === null ? 'never appeared within ' + tmo + ' ms' : 'after ' + mm(errAt)} (expected 15-17 min; window accepted 13-18 min); Failing card=${d?.cards[1].value} (expected 1); ${fmt(r)} (expected streak 3 and highlighted row); "consecutive_failures=3" lines in HFS log=${log === null ? 'n/a' : log}`);
    });
    await guard('13.4e', async () => {
      const seen = [];
      for (const [key, label] of [['status', 'Status'], ['sent', 'Most sent'], ['fails', 'Fail streak']]) {
        await page.goto(`${BASE}/ui/subscriptions`, { waitUntil: 'load' });
        await page.click('.subs-sort summary');
        if (key === 'status') await shot(page, S, '13.4e-sort-menu');
        await Promise.all([page.waitForURL(`**/ui/subscriptions?sort=${key}`, { timeout: 10000 }), page.click(`.subs-sort a.menu__option[href$="sort=${key}"]`)]);
        const cur = (await page.locator('.subs-sort summary span').nth(1).innerText()).trim();
        const rows = await page.locator('tbody tr').count();
        seen.push(`${key}: label="${cur}" rows=${rows}`);
        if (cur !== label) seen.push(`MISMATCH expected ${label}`);
      }
      await shot(page, S, '13.4e-sort-fails');
      record(S, '13.4e', seen.some((s) => s.includes('MISMATCH')) ? 'fail' : 'pass', seen.join('; '));
    });
    await guard('13.4f', async () => {
      // after the 3rd window: status=error; recovery is a reactivation (PUT status=requested), restarting the receiver alone does not recover
      await H.startReceiver();
      await H.sleep(70000);
      const still = row(await H.scrapeSubs(page));
      const sub = (await api('/Subscription/manual-sub')).json;
      const apiStatus = sub?.status;
      let reactivated = null, rs = null, after;
      if (sub) {
        sub.status = 'requested';
        const put = await api('/Subscription/manual-sub', { method: 'PUT', headers: { 'content-type': 'application/fhir+json' }, body: JSON.stringify(sub), save: 't9-13.4f-reactivate.json' });
        rs = put.status;
        for (let i = 0; i < 30; i++) { await H.sleep(1000); const s = (await api('/Subscription/manual-sub')).json?.status; if (s === 'active') { reactivated = true; break; } }
      }
      await H.sleep(2000);
      after = row(await H.scrapeSubs(page));
      await shot(page, S, '13.4f-reactivated');
      const ok = apiStatus === 'error' && reactivated && /^Active$/.test(after?.status) && after.streak === '0' && H.num(after.sent) <= 1;
      record(S, '13.4f', ok ? 'pass' : 'fail', `after the third window with the receiver back for 70 s: stored status=${apiStatus} (expected error), UI row "${still?.status}"; PUT status=requested -> HTTP ${rs}, stored status active=${reactivated}; UI after: ${fmt(after)} (expected Active, streak 0, Sent restarts at 0 or 1)`);
    });
  }

  // 13.5 restart with rehydration
  await guard('13.5', async () => {
    const cmd = normalCmd();
    if (!cmd) { record(S, '13.5', 'na', 'neither HFS_RESTART_CMD nor HFS_STOP_CMD+HFS_START_CMD provided'); return; }
    const before = H.hfsLogCount(/Failed to persist subscription status transition/g);
    const pre = row(await H.scrapeSubs(page));
    const r = H.sh(cmd, 300000);
    const up = await H.waitServer(180000);
    await H.sleep(5000);
    const d = await H.scrapeSubs(page);
    await shot(page, S, '13.5-after-restart');
    const rr = row(d);
    const after = H.hfsLogCount(/Failed to persist subscription status transition/g);
    const stored = (await api('/Subscription/manual-sub')).json?.status;
    const ok = r.ok && up && rr && /^Active$/.test(rr.status) && rr.sent === '0' && d.cards[3].value === '0' && (after === null || after === 0) && stored === 'active';
    record(S, '13.5', ok ? 'pass' : 'fail', `restart cmd ok=${r.ok}, server up=${up}; before: ${fmt(pre)}; after: ${fmt(rr)}; Delivered card=${d.cards[3].value} (expected 0); Active card=${d.cards[0].value}; stored status=${stored}; "Failed to persist subscription status transition" in HFS log=${after === null ? 'n/a (HFS_LOG not set)' : after} (expected 0)`);
  });

  // 13.6 engine disabled
  await guard('13.6', async () => {
    const cmd = disabledCmd();
    if (!cmd) { record(S, '13.6', 'na', 'neither HFS_RESTART_DISABLED_CMD nor HFS_STOP_CMD+HFS_START_CMD provided'); return; }
    const r = H.sh(cmd, 300000);
    const up = await H.waitServer(180000);
    const d = await H.scrapeSubs(page);
    await shot(page, S, '13.6-engine-disabled');
    const ok = r.ok && up && d.unavailable && /The subscriptions engine is not enabled on this server\./.test(d.notice) && /HFS_SUBSCRIPTIONS_ENABLED=true/.test(d.notice) && d.sidebar && d.rows.length === 0;
    record(S, '13.6', ok ? 'pass' : 'fail', `restart ok=${r.ok}, up=${up}; notice="${d.notice}"; sidebar entry present=${d.sidebar}; table rows=${d.rows.length}; expected the notice naming HFS_SUBSCRIPTIONS_ENABLED=true and the sidebar entry`);
    // restore the normal configuration so later steps find the engine on
    const rc = normalCmd();
    if (rc) { H.sh(rc, 300000); const back = await H.waitServer(180000); await H.sleep(4000); const e = await H.scrapeSubs(page); record(S, '13.6-restore', back && !e.unavailable ? 'pass' : 'fail', `engine restored with HFS_RESTART_CMD: up=${back}, page available=${!e.unavailable}`); }
  });

  // 13.7 console errors
  const ce = consoleErrors.filter((e) => !/favicon|Failed to load resource/i.test(e));
  record(S, '13.7', ce.length === 0 ? 'pass' : 'fail', ce.length ? ce.join(' | ').slice(0, 500) : 'no browser console errors on /ui/subscriptions and the Batch page (resource-load errors from the intentionally stopped server excluded)');

  // 13.8 usable without JavaScript
  await guard('13.8', async () => {
    const live = await H.scrapeSubs(page);
    if (live.unavailable) { record(S, '13.8', 'na', 'engine page not available'); return; }
    const { browser: b2, context, page: p2 } = await launch({ javaScriptEnabled: false });
    await p2.goto(`${BASE}/ui/subscriptions`, { waitUntil: 'load' });
    const vals = await p2.locator('.stat__value').allInnerTexts();
    const rowTxt = (await p2.locator('tbody tr').first().innerText()).replace(/\s+/g, ' ');
    await shot(p2, S, '13.8-no-javascript');
    await b2.close();
    const same = JSON.stringify(vals.map((v) => v.trim())) === JSON.stringify(live.cards.map((c) => c.value)) && rowTxt.includes('manual-sub');
    record(S, '13.8', same ? 'pass' : 'fail', `JS disabled: card values=[${vals.map((v) => v.trim())}] vs JS enabled=[${live.cards.map((c) => c.value)}]; first row="${rowTxt}"`);
  });

  // 13.9 main dashboard glance
  await guard('13.9', async () => {
    await page.goto(`${BASE}/ui`, { waitUntil: 'load' });
    await page.waitForTimeout(2500);
    const txt = (await page.locator('main').first().innerText().catch(() => '')).replace(/\s+/g, ' ');
    await shot(page, S, '13.9-main-dashboard');
    const enc = await api('/Encounter?_summary=count', { save: 't9-13.9-encounter-count.json' });
    record(S, '13.9', txt.length > 20 ? 'pass' : 'fail', `/ui rendered (${txt.length} chars); server Encounter total=${enc.json?.total}; stat cards text: ${txt.slice(0, 300)}. Compare against the imported counts + T2/T4/T5/T8 creations by eye on the screenshot.`);
  });
} finally {
  await browser.close();
}
process.exit(0); // detached receiver / sockets can otherwise keep node alive
