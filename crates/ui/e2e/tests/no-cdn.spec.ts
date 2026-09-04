import { test, expect } from "../pages/fixtures";
import { ROUTES, seedBulkImportDetail } from "../pages/routes";
import { VdEditor } from "../pages/vd-editor";

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

// #821: the general sweep above visits every route once, `networkidle` —
// enough to catch a page's own initial requests, but not the ones the
// ViewDefinition editor's lint and completion fire *while typing*, well
// after the page has gone idle. This exercises a whole typing session (an
// edit that triggers the lint's own server round trip, then a completion
// popup) and checks every request it produces the same way.

test("a ViewDefinition editing session (lint + completion) makes no external-origin request", async ({
  page,
  baseURL,
}) => {
  const origin = new URL(baseURL!).origin;
  const foreign: string[] = [];
  page.on("request", (r) => {
    const u = r.url();
    if (u.startsWith("data:") || u.startsWith("blob:")) return;
    if (!u.startsWith(origin)) foreign.push(`${r.method()} ${u}`);
  });

  await page.goto("/ui/sql/view-definitions?vd=new");
  const ed = new VdEditor(page);
  const doc = `{
  "resourceType": "ViewDefinition",
  "status": "active",
  "resource": "Patient",
  "select": [
    {
      "column": [{ "name": "id", "path": "Patient.name" }]
    }
  ]
}`;
  // The edit itself fires the lint's server round trip (`setDoc` waits for
  // it — see its own doc comment).
  await ed.setDoc(doc);

  // A completion request too: right before the closing quote of
  // "Patient.name", typing "." opens the popup.
  await ed.setCursorAfter(doc, "Patient.name");
  await page.keyboard.type(".");
  await expect(ed.completionPopup).toBeVisible();

  expect(foreign, `off-origin requests:\n${foreign.join("\n")}`).toEqual([]);
});
