import { test, expect } from "../pages/fixtures";
import { ROUTES, seedBulkImportDetail } from "../pages/routes";

// crates/ui/README.md states "rules of the road" that were enforced only by
// review. These make three of them executable, across every full page. The
// route list is shared with the other cross-page guards (#543).

test("no page makes an external-origin request (no CDN)", async ({ page, baseURL, request }) => {
  const origin = new URL(baseURL!).origin;
  const foreign: string[] = [];
  page.on("request", (r) => {
    const u = r.url();
    if (u.startsWith("data:") || u.startsWith("blob:")) return;
    if (!u.startsWith(origin)) foreign.push(`${r.method()} ${u}`);
  });
  for (const route of [...ROUTES, await seedBulkImportDetail(request)]) {
    await page.goto(route, { waitUntil: "networkidle" });
  }
  expect(foreign, `off-origin requests:\n${foreign.join("\n")}`).toEqual([]);
});

test("no route emits an uncaught page error", async ({ page, request }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  for (const route of [...ROUTES, await seedBulkImportDetail(request)]) {
    await page.goto(route, { waitUntil: "networkidle" });
  }
  expect(errors, errors.join("\n")).toEqual([]);
});

test("rendered HTML carries no inline executable <script>", async ({ page, request }) => {
  for (const route of [...ROUTES, await seedBulkImportDetail(request)]) {
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
