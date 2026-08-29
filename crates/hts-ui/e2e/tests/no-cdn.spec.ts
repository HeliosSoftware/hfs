import { expect, test } from "@playwright/test";

// D5/D6 guardrail for HTS Phase 1. Mirrors crates/ui/e2e/tests/no-cdn.spec.ts
// so the "no off-origin requests, no inline scripts, no uncaught page errors"
// invariants that `hts-ui-design.md` §1 / §11.2 promise are actually enforced
// at CI time and not just aspirational. Design-doc §12 Phase 1 acceptance
// row `axe + nojs + no-cdn green` becomes real coverage here.
//
// The shared enumerator refactor into `helios-ui-chrome` (Phase 8) can
// still consolidate this file with the sibling `crates/ui/e2e/tests/no-cdn.spec.ts`
// later; until then the ~35 LOC duplication is intentional and cheap.

const ROUTES = [
  "/ui/hts",
  "/ui/hts/code-systems",
  "/ui/hts/value-sets",
  "/ui/hts/concept-maps",
  "/ui/hts/operations",
  "/ui/hts/import",
  "/ui/hts/capability-statement",
  // Detail pages: seed.mjs provisions ex-cs-1, ex-vs-1, ex-cm-1, and the
  // §8.3 operation-first landing redirects /{id} to /{id}/{default-op}.
  // Hitting the effective landing URL directly avoids Playwright following
  // a 308 mid-request and dropping the network capture.
  "/ui/hts/code-systems/ex-cs-1/lookup",
  "/ui/hts/value-sets/ex-vs-1/expand",
  "/ui/hts/concept-maps/ex-cm-1/translate",
];

test("no page makes an external-origin request (no CDN)", async ({ page, baseURL }) => {
  const origin = new URL(baseURL!).origin;
  const foreign: string[] = [];
  page.on("request", (r) => {
    const u = r.url();
    if (u.startsWith("data:") || u.startsWith("blob:")) return;
    if (!u.startsWith(origin)) foreign.push(`${r.method()} ${u}`);
  });
  for (const route of ROUTES) {
    await page.goto(route, { waitUntil: "networkidle" });
  }
  expect(foreign, `off-origin requests:\n${foreign.join("\n")}`).toEqual([]);
});

test("no route emits an uncaught page error", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  for (const route of ROUTES) {
    await page.goto(route, { waitUntil: "networkidle" });
  }
  expect(errors, errors.join("\n")).toEqual([]);
});

test("rendered HTML carries no inline executable <script>", async ({ page }) => {
  for (const route of ROUTES) {
    await page.goto(route, { waitUntil: "domcontentloaded" });
    const inline = await page.$$eval("script", (nodes) =>
      nodes
        .filter((n) => {
          if (n.getAttribute("src")) return false;
          if ((n.textContent ?? "").trim().length === 0) return false;
          // Data carriers are inert: browsers never execute them and CSP
          // script rules don't apply. Matches the HFS spec's carve-out.
          const type = (n.getAttribute("type") ?? "").toLowerCase();
          return type !== "application/json" && type !== "application/ld+json";
        })
        .map((n) => (n.textContent ?? "").slice(0, 80)),
    );
    expect(inline, `inline scripts on ${route}:\n${inline.join("\n")}`).toEqual([]);
  }
});
