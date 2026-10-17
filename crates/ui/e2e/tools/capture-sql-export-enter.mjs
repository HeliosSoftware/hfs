#!/usr/bin/env node
// Visual evidence for Enter in "Filter subjects" on /ui/sql/export/new (#1665).
//
// Seeds three parameterless ViewDefinitions, opens New SQL Export, ticks one,
// types a filter into "Filter subjects" and presses Enter — the habit the
// issue describes. Before the fix the browser's implicit submission posts the
// export form and lands on the job list with a job the user never asked for;
// after it, the builder stays put with the filter and selection intact. Run
// it once against the code before a change and once after, into separate
// directories, and compare.
//
//   HFS_E2E_BASE_URL=http://127.0.0.1:18665 \
//     node crates/ui/e2e/tools/capture-sql-export-enter.mjs --out docs/images/1665/before
//
// Writes, into --out:
//   filter-typed.png  whole viewport: subject ticked, filter typed, Enter not yet pressed
//   after-enter.png   whole viewport once Enter has had its effect
//   enter.gif         the sequence, scaled down for inline viewing
//   enter.mp4         the sequence, whole viewport
//
// Also prints the URL after Enter and how many jobs the per-user settings
// store holds. Needs a fresh server (the seeded names are fixed) and ffmpeg
// on PATH for the gif and mp4.

import { chromium, request as apiRequest } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

function arg(name, fallback) {
  const i = process.argv.indexOf(`--${name}`);
  return i > -1 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

const base = (arg("base", process.env.HFS_E2E_BASE_URL) || "http://127.0.0.1:8080").replace(/\/$/, "");
const out = resolve(arg("out", "sql-export-enter-evidence"));
const filter = "female";
// Where the pointer rests: empty page, away from the sidebar (which expands
// over the content on hover).
const idle = { x: 1200, y: 760 };
const viewport = { width: 1280, height: 800 };

mkdirSync(out, { recursive: true });
const scratch = mkdtempSync(join(tmpdir(), "hfs-sql-export-enter-evidence-"));

const api = await apiRequest.newContext({ baseURL: base });
const names = ["patients_female", "patients_male", "patients_all"];
const ids = [];
for (const name of names) {
  const res = await api.post("/ViewDefinition", {
    headers: { "Content-Type": "application/fhir+json" },
    data: {
      resourceType: "ViewDefinition",
      name,
      status: "active",
      resource: "Patient",
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    },
  });
  if (!res.ok()) throw new Error(`seeding ${name}: ${res.status()} ${await res.text()}`);
  ids.push((await res.json()).id);
}

const browser = await chromium.launch();
const context = await browser.newContext({
  viewport,
  deviceScaleFactor: 2,
  colorScheme: "light",
  recordVideo: { dir: scratch, size: viewport },
});
const videoStart = Date.now();
const page = await context.newPage();
const recording = page.video();

let startAt;
let endAt;
try {
  await page.goto(`${base}/ui/sql/export/new`, { waitUntil: "networkidle" });
  await page.mouse.move(idle.x, idle.y);
  startAt = Date.now();
  await page.waitForTimeout(800);
  await page.locator(`input[name="subject"][value="ViewDefinition/${ids[0]}"]`).check();
  await page.mouse.move(idle.x, idle.y);
  await page.waitForTimeout(500);
  const filterInput = page.locator(".card-head__tools--subjects input[type='search']");
  await filterInput.pressSequentially(filter, { delay: 90 });
  await page.waitForTimeout(700);
  await page.screenshot({ path: join(out, "filter-typed.png") });

  await filterInput.press("Enter");
  // Either the form posts and the page navigates, or nothing happens; give
  // the navigation, if any, time to finish before photographing.
  await page.waitForTimeout(1500);
  await page.waitForLoadState("networkidle");
  await page.mouse.move(idle.x, idle.y);
  await page.waitForTimeout(500);
  await page.screenshot({ path: join(out, "after-enter.png") });
  endAt = Date.now();

  const settings = await (await api.get("/_user/settings")).json();
  console.log(
    "after Enter:",
    JSON.stringify({
      url: new URL(page.url()).pathname,
      jobs: Object.keys(settings.sqlExport?.jobs ?? {}).length,
    }),
  );
} finally {
  await context.close(); // flushes the video
  await browser.close();
  await api.dispose();
}

const video = await recording.path();
const start = Math.max(0, (startAt - videoStart) / 1000);
const length = ((endAt - startAt) / 1000).toFixed(2);

execFileSync(
  "ffmpeg",
  [
    "-y", "-loglevel", "error", "-ss", start.toFixed(2), "-t", length, "-i", video,
    "-vf",
    "fps=8,scale=800:-1:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4",
    "-loop", "0", join(out, "enter.gif"),
  ],
  { stdio: "inherit" },
);
execFileSync(
  "ffmpeg",
  [
    "-y", "-loglevel", "error", "-ss", start.toFixed(2), "-t", length, "-i", video,
    "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "23", "-movflags", "+faststart",
    "-an", join(out, "enter.mp4"),
  ],
  { stdio: "inherit" },
);

rmSync(scratch, { recursive: true, force: true });
console.log(`evidence written to ${out}`);
