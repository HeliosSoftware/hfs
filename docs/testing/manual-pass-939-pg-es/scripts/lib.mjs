// Shared helper for the #939 pg-es manual pass. Screenshots are numbered, 1280x800, and logged to shots.tsv.
import { chromium } from 'playwright-core';
import fs from 'fs';
import path from 'path';

export const BASE = process.env.HFS_BASE || (() => { throw new Error('HFS_BASE is required (no default: the real pg-es server must never be hit by accident)'); })();
export const OUT = process.env.OUT || `${process.env.HOME}/tmp/939/out`;
export const SHOTS = `${OUT}/shots`;
export const OUTPUTS = `${OUT}/outputs`;
for (const d of [SHOTS, OUTPUTS]) fs.mkdirSync(d, { recursive: true });

const counters = {};
const root = `${process.env.HOME}/.cache/ms-playwright/chromium-1243/`;
const exe = fs.readdirSync(root).map((d) => `${root}${d}/chrome`).find((p) => fs.existsSync(p));

export async function launch(opts = {}) {
  const browser = await chromium.launch({ executablePath: exe });
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 }, acceptDownloads: true, ...opts });
  const page = await context.newPage();
  return { browser, context, page };
}

// shot(page, 'T4', '4.1a-query-results') -> shots/T4-4.1a-query-results-01.png (full page is cropped to the first 800px unless full:true)
export async function shot(page, step, name, { full = false } = {}) {
  const key = `${step}-${name}`;
  counters[key] = (counters[key] || 0) + 1;
  const file = `${step}-${name}-${String(counters[key]).padStart(2, '0')}.png`;
  await page.screenshot({ path: path.join(SHOTS, file), fullPage: full });
  fs.appendFileSync(`${OUT}/shots.tsv`, `${new Date().toISOString()}\t${file}\n`);
  return file;
}

// record('T4', '4.1', 'pass'|'fail'|'na', 'note') -> appends to steps.tsv
export function record(step, id, result, note = '') {
  fs.appendFileSync(`${OUT}/steps.tsv`, `${new Date().toISOString()}\t${step}\t${id}\t${result}\t${note.replace(/\s+/g, ' ')}\n`);
  console.log(`[${step} ${id}] ${result} ${note}`);
}

// api(path, {method, headers, body, save:'t4-xxx.json'}) -> {status, headers, text, json}; body saved under outputs/ when save is set
export async function api(p, { method = 'GET', headers = {}, body, save } = {}) {
  const res = await fetch(p.startsWith('http') ? p : BASE + p, { method, headers, body });
  const text = await res.text();
  let json; try { json = JSON.parse(text); } catch {}
  if (save) fs.writeFileSync(path.join(OUTPUTS, save), `# ${method} ${p} -> ${res.status}\n${text}`);
  return { status: res.status, headers: Object.fromEntries(res.headers), text, json };
}
