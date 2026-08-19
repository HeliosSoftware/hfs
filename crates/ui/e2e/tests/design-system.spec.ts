import { test, expect } from "../pages/fixtures";
import type { APIRequestContext, Page } from "@playwright/test";
import { ROUTES, seedBulkImportDetail } from "../pages/routes";
import postcss from "postcss";

// The design-system guard (#543). The stylesheet defines one component
// vocabulary (crates/ui/README.md, "Component vocabulary"); these tests make
// divergence executable instead of a review judgement:
//   - a class that matches no rule is a typo or a second vocabulary starting
//     (class="table" shipped that way and styled nothing);
//   - a page <h1> off the canonical class is a second heading scale;
//   - primary buttons that resolve to different metrics on different pages
//     are two button designs;
//   - a selector defined twice renders as the cascade-merge of two authors'
//     blocks, which nobody wrote.

// Classes added at runtime by vendored libraries, not defined in app.css.
const RUNTIME_CLASSES = /^htmx-/;

// The assets are read over HTTP from the server under test, not from the
// source tree: the CI runner drives a packaged binary with no checkout
// alongside it, and in HFS_E2E_BASE_URL mode the running server is the only
// truth worth checking anyway.
async function fetchAsset(request: APIRequestContext, path: string): Promise<string> {
  const res = await request.get(path);
  if (!res.ok()) throw new Error(`${path} -> ${res.status()}`);
  return res.text();
}

async function stylesheet(request: APIRequestContext): Promise<string> {
  return fetchAsset(request, "/ui/assets/app.css");
}

// A class is load-bearing if a rule styles it — or if shipped script selects
// it (a JS hook like .json-line--foldable). Only dot-prefixed names inside JS
// string literals count, so createElement("table") never legitimizes a dead
// class="table". The script list comes from the served pages themselves
// (every page loads its scripts via the shared layout).
async function definedClasses(page: Page, request: APIRequestContext): Promise<Set<string>> {
  const found = new Set<string>();
  const css = (await stylesheet(request)).replace(/\/\*[\s\S]*?\*\//g, "");
  for (const m of css.matchAll(/\.(-?[A-Za-z_][A-Za-z0-9_-]*)/g)) found.add(m[1]);

  await page.goto("/ui", { waitUntil: "domcontentloaded" });
  const sources = await page.$$eval("script[src]", (nodes) =>
    nodes.map((n) => n.getAttribute("src") ?? "").filter((s) => s.startsWith("/ui/assets/")),
  );
  for (const src of sources) {
    if (src.endsWith("htmx.min.js")) continue;
    const js = await fetchAsset(request, src);
    for (const str of js.matchAll(/"(?:[^"\\\n]|\\.)*"|'(?:[^'\\\n]|\\.)*'|`(?:[^`\\]|\\.)*`/g)) {
      for (const m of str[0].matchAll(/\.(-?[A-Za-z_][A-Za-z0-9_-]*)/g)) found.add(m[1]);
    }
  }
  return found;
}

test("every class used on every page matches a rule in app.css", async ({ page, request }) => {
  const defined = await definedClasses(page, request);
  const offenders: string[] = [];
  for (const route of [...ROUTES, await seedBulkImportDetail(request)]) {
    await page.goto(route, { waitUntil: "networkidle" });
    const used = await page.evaluate(() =>
      Array.from(new Set(Array.from(document.querySelectorAll("*")).flatMap((el) => Array.from(el.classList)))),
    );
    for (const cls of used) {
      if (RUNTIME_CLASSES.test(cls)) continue;
      if (!defined.has(cls)) offenders.push(`${route}: .${cls}`);
    }
  }
  expect(
    offenders,
    `classes with no matching rule in app.css (typo, or a second vocabulary starting):\n${offenders.join("\n")}`,
  ).toEqual([]);
});

test("every page <h1> uses the canonical .page-head__title", async ({ page, request }) => {
  const offenders: string[] = [];
  for (const route of [...ROUTES, await seedBulkImportDetail(request)]) {
    await page.goto(route, { waitUntil: "domcontentloaded" });
    const headings = await page.$$eval("h1", (nodes) =>
      nodes.map((n) => ({ classes: n.className, text: (n.textContent ?? "").trim().slice(0, 40) })),
    );
    expect(headings.length, `${route} should have exactly one <h1>`).toBe(1);
    for (const h of headings) {
      if (!h.classes.split(/\s+/).includes("page-head__title")) {
        offenders.push(`${route}: <h1 class="${h.classes}"> (“${h.text}”)`);
      }
    }
  }
  expect(offenders, `page titles off the canonical class:\n${offenders.join("\n")}`).toEqual([]);
});

test("primary buttons resolve to the same metrics on every page", async ({ page, request }) => {
  // Computed, not declared: a page-layer override that restyles the shared
  // control shows up here no matter how it was written.
  const byRoute = new Map<string, string[]>();
  for (const route of [...ROUTES, await seedBulkImportDetail(request)]) {
    await page.goto(route, { waitUntil: "networkidle" });
    const signatures = await page.$$eval(".btn--primary", (nodes) =>
      nodes
        .filter((n) => n instanceof HTMLElement && n.offsetParent !== null)
        .map((n) => {
          const s = getComputedStyle(n);
          return `height=${s.height} radius=${s.borderRadius} bg=${s.backgroundColor} color=${s.color}`;
        }),
    );
    if (signatures.length) byRoute.set(route, Array.from(new Set(signatures)));
  }
  const distinct = new Set(Array.from(byRoute.values()).flat());
  const listing = Array.from(byRoute, ([r, sigs]) => `${r}: ${sigs.join(" | ")}`).join("\n");
  expect(distinct.size, `primary buttons diverge across pages:\n${listing}`).toBeLessThanOrEqual(1);
});

test("no selector is defined twice in app.css", async ({ request }) => {
  const css = await stylesheet(request);
  const root = postcss.parse(css);
  const seen = new Map<string, number[]>();
  root.walkRules((rule) => {
    // Context = enclosing at-rules EXCEPT @layer: the layer a rule lives in
    // must not license a second definition, but the same selector inside a
    // different @media block is a legitimate refinement.
    const ctx: string[] = [];
    let p: any = rule.parent;
    while (p && p.type === "atrule") {
      if (p.name !== "layer") ctx.push(`@${p.name} ${p.params}`);
      p = p.parent;
    }
    const key =
      ctx.reverse().join(" | ") +
      " :: " +
      rule.selectors.map((s) => s.replace(/\s+/g, " ").trim()).sort().join(", ");
    if (!seen.has(key)) seen.set(key, []);
    seen.get(key)!.push(rule.source!.start!.line);
  });
  const dupes = Array.from(seen)
    .filter(([, lines]) => lines.length > 1)
    .map(([key, lines]) => `${key} — lines ${lines.join(", ")}`);
  expect(dupes, `selectors defined more than once:\n${dupes.join("\n")}`).toEqual([]);
});
