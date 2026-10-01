#!/usr/bin/env node
// Visual evidence for the busy Execute button on /ui/batch (#1253).
//
// Drives a running HFS through the Batch/Transaction page and records what a
// reviewer needs to judge the busy state: the resting footer, the footer
// while the Execute request is in flight, and the rendered response, plus a
// short clip of the whole sequence. Run it once against the code before a
// change and once after, into separate directories, and compare.
//
//   HFS_E2E_BASE_URL=http://127.0.0.1:18253 \
//     node crates/ui/e2e/tools/capture-busy-button.mjs --out docs/images/1253/before
//
// Writes, into --out:
//   before-click.png       Execute button crop, preflight rendered, nothing pressed
//   before-click-page.png  the same moment, whole viewport
//   busy.png               Execute button crop while the request is in flight
//   busy-page.png          the same moment, whole viewport
//   after-response.png     whole viewport once the response has rendered
//   busy.gif               the sequence, cropped to the Execute button
//   busy.mp4               the sequence, whole viewport
//
// The Execute POST is parked in page.route for --hold-ms so the busy state
// lasts long enough to photograph and to watch; the request then continues
// to the real server. Needs ffmpeg on PATH for the gif and mp4.

import { chromium } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

function arg(name, fallback) {
  const i = process.argv.indexOf(`--${name}`);
  return i > -1 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

const base = (arg("base", process.env.HFS_E2E_BASE_URL) || "http://127.0.0.1:8080").replace(/\/$/, "");
const out = resolve(arg("out", "busy-button-evidence"));
const holdMs = Number(arg("hold-ms", "2500"));
const restMs = 1200; // how long the clip shows the resting label before the click
// CSS px of page kept around the Execute button in crops. The footer is
// space-between, so the button is flush right: extra room on the left keeps
// a busy state that widens the button inside the same frame.
const padX = 48;
const padY = 18;
// Where the pointer rests: empty page, away from the sidebar (which expands
// over the content on hover) and from the button (hover changes its fill).
const idle = { x: 1200, y: 760 };
const viewport = { width: 1280, height: 800 };

mkdirSync(out, { recursive: true });
const scratch = mkdtempSync(join(tmpdir(), "hfs-busy-evidence-"));

// Two POST Patient entries, the same shape the Playwright batch spec uses.
const stamp = Date.now();
const bundlePath = join(scratch, `busy-evidence-bundle-${stamp}.json`);
writeFileSync(
  bundlePath,
  JSON.stringify({
    resourceType: "Bundle",
    type: "batch",
    entry: [
      {
        fullUrl: "urn:uuid:00000000-0000-4000-8000-000000000001",
        resource: { resourceType: "Patient", name: [{ family: `BusyEvidence${stamp}` }] },
        request: { method: "POST", url: "Patient" },
      },
      {
        fullUrl: "urn:uuid:00000000-0000-4000-8000-000000000002",
        resource: { resourceType: "Patient", name: [{ family: `BusyEvidenceB${stamp}` }] },
        request: { method: "POST", url: "Patient" },
      },
    ],
  }),
);

const browser = await chromium.launch();
const context = await browser.newContext({
  viewport,
  deviceScaleFactor: 2,
  colorScheme: "light",
  reducedMotion: "no-preference",
  recordVideo: { dir: scratch, size: viewport },
});
const videoStart = Date.now();
const page = await context.newPage();
const recording = page.video();

let release;
const parked = new Promise((r) => { release = r; });
await page.route((url) => url.pathname === "/", async (route) => {
  if (route.request().method() !== "POST") return route.continue().catch(() => {});
  await parked;
  await route.continue().catch(() => {});
});

const execute = page.locator("#batch-execute-top");

async function buttonClip() {
  const box = await execute.boundingBox();
  if (!box) throw new Error("the Execute button is not rendered");
  const x = Math.max(0, Math.floor(box.x - padX));
  const y = Math.max(0, Math.floor(box.y - padY));
  // Even sizes: libx264/yuv420p and the gif scaler both want them.
  const even = (n) => n - (n % 2);
  return {
    x,
    y,
    width: even(Math.min(viewport.width - x, Math.ceil(box.width + 2 * padX))),
    height: even(Math.min(viewport.height - y, Math.ceil(box.height + 2 * padY))),
  };
}

let clip;
let clickAt;
let releaseAt;
try {
  await page.goto(`${base}/ui/batch`, { waitUntil: "networkidle" });
  await page.locator("#batch-file").setInputFiles(bundlePath);
  await page.locator("#batch-preflight").waitFor({ state: "visible" });
  await execute.waitFor({ state: "visible" });
  // Nothing hovered or focused: the resting button as a reviewer first sees it.
  await page.mouse.move(idle.x, idle.y);

  clip = await buttonClip();
  await page.screenshot({ path: join(out, "before-click.png"), clip });
  await page.screenshot({ path: join(out, "before-click-page.png") });
  await page.waitForTimeout(restMs);

  clickAt = Date.now();
  await execute.click();
  await page.waitForFunction(
    () => document.getElementById("batch-execute-top")?.getAttribute("aria-busy") === "true",
  );
  // Move off the button so the capture is the busy style, not the hover style.
  await page.mouse.move(idle.x, idle.y);
  await page.waitForTimeout(300);
  await page.screenshot({ path: join(out, "busy.png"), clip });
  await page.screenshot({ path: join(out, "busy-page.png") });

  const busyState = await execute.evaluate((b) => ({
    ariaBusy: b.getAttribute("aria-busy"),
    disabled: b.disabled,
    label: b.textContent.trim(),
    color: getComputedStyle(b).color,
    ring: getComputedStyle(b, "::after").content,
  }));
  console.log("busy state:", JSON.stringify(busyState));

  await page.waitForTimeout(Math.max(0, holdMs - (Date.now() - clickAt)));
  releaseAt = Date.now();
  release();
  await page.locator("#batch-response").waitFor({ state: "visible" });
  await page.waitForTimeout(600);
  await page.screenshot({ path: join(out, "after-response.png") });
} finally {
  release();
  await context.close(); // flushes the video
  await browser.close();
}

const video = await recording.path();
const start = Math.max(0, (clickAt - videoStart - restMs) / 1000);
const end = (releaseAt - videoStart + 400) / 1000;
const length = (end - start).toFixed(2);

// Button crop, enlarged 3x so the label and ring are legible inline.
const crop = `crop=${clip.width}:${clip.height}:${clip.x}:${clip.y}`;
execFileSync(
  "ffmpeg",
  [
    "-y", "-loglevel", "error", "-ss", start.toFixed(2), "-t", length, "-i", video,
    "-vf",
    `${crop},fps=15,scale=iw*3:-1:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4`,
    "-loop", "0", join(out, "busy.gif"),
  ],
  { stdio: "inherit" },
);
execFileSync(
  "ffmpeg",
  [
    "-y", "-loglevel", "error", "-ss", start.toFixed(2), "-t", length, "-i", video,
    "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "23", "-movflags", "+faststart",
    "-an", join(out, "busy.mp4"),
  ],
  { stdio: "inherit" },
);

rmSync(scratch, { recursive: true, force: true });
console.log(`evidence written to ${out}`);
