import { test, expect } from "../pages/fixtures";

// crates/ui/README.md states "rules of the road" that were enforced only by
// review. These make three of them executable, across every full page.
const ROUTES = [
  "/ui",
  "/ui/resources",
  "/ui/batch",
  "/ui/compartments",
  "/ui/search-parameters",
  "/ui/queries",
  "/ui/history",
  "/ui/search",
  "/ui/tenants",
  "/ui/editor?type=Patient",
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
          // script rules don't apply. The i18n message blob (#395) rides one.
          const type = (n.getAttribute("type") ?? "").toLowerCase();
          return type !== "application/json" && type !== "application/ld+json";
        })
        .map((n) => (n.textContent ?? "").slice(0, 80)),
    );
    expect(inline, `inline scripts on ${route}:\n${inline.join("\n")}`).toEqual([]);
  }
});
